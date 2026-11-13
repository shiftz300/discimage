// SPDX-License-Identifier: GPL-3.0-or-later
// MMC user-client handshake adapted from NaiveTomcat/discprobe-macos,
// tools/mac/discprobe-spt.c. DiscImage owns selection, bounds and all fitting.
#include <IOKit/IOCFPlugIn.h>
#include <IOKit/scsi/SCSITaskLib.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include "registry.h"

struct Probe {
    IOCFPlugInInterface **plugin;
    MMCDeviceInterface **mmc;
    SCSITaskDeviceInterface **device;
    bool exclusive;
    bool recording;
};

static void release_probe(struct Probe *probe) {
    if (probe->device) (*probe->device)->Release(probe->device);
    if (probe->mmc) (*probe->mmc)->Release(probe->mmc);
    if (probe->plugin) IODestroyPlugInInterface(probe->plugin);
    free(probe);
}

static bool is_optical_service(io_registry_entry_t entry, const void *context) {
    (void)context;
    return IOObjectConformsTo(entry, "IOCompactDiscServices") ||
           IOObjectConformsTo(entry, "IODVDServices") ||
           IOObjectConformsTo(entry, "IOBDServices");
}

static io_service_t optical_service(const char *bsd) {
    if (strncmp(bsd, "IOService:", 10) == 0) {
        io_service_t service = IORegistryEntryFromPath(kIOMainPortDefault, bsd);
        if (service && is_optical_service(service, NULL)) return service;
        if (service) IOObjectRelease(service);
        return IO_OBJECT_NULL;
    }
    CFMutableDictionaryRef match = IOBSDNameMatching(kIOMainPortDefault, 0, bsd);
    if (!match) return IO_OBJECT_NULL;
    io_registry_entry_t entry = IOServiceGetMatchingService(kIOMainPortDefault, match);
    if (!entry) return IO_OBJECT_NULL;
    io_service_t service = IO_OBJECT_NULL;
    if (whole_media(entry) && IOObjectConformsTo(entry, "IOMedia"))
        service = matching_parent(entry, is_optical_service, NULL);
    IOObjectRelease(entry);
    return service;
}

static void *open_optical(const char *bsd, int32_t *error, bool exclusive, bool recording) {
    *error = kIOReturnNotFound;
    io_service_t service = optical_service(bsd);
    if (!service) return NULL;
    struct Probe *probe = calloc(1, sizeof(*probe));
    if (!probe) {
        IOObjectRelease(service);
        *error = kIOReturnNoMemory;
        return NULL;
    }
    SInt32 score = 0;
    *error = IOCreatePlugInInterfaceForService(service, kIOMMCDeviceUserClientTypeID,
                                              kIOCFPlugInInterfaceID, &probe->plugin, &score);
    IOObjectRelease(service);
    if (*error == kIOReturnSuccess && probe->plugin &&
        (*probe->plugin)->QueryInterface(probe->plugin, CFUUIDGetUUIDBytes(kIOMMCDeviceInterfaceID),
                                         (LPVOID *)&probe->mmc) == S_OK && probe->mmc) {
        probe->device = (*probe->mmc)->GetSCSITaskDeviceInterface(probe->mmc);
    }
    *error = !probe->device ? kIOReturnUnsupported : exclusive
        ? (*probe->device)->ObtainExclusiveAccess(probe->device) : kIOReturnSuccess;
    if (*error != kIOReturnSuccess) {
        release_probe(probe);
        return NULL;
    }
    probe->exclusive = exclusive;
    probe->recording = recording;
    return probe;
}

void *discimage_probe_open(const char *bsd, int32_t *error, bool exclusive) {
    return open_optical(bsd, error, exclusive, false);
}

void *discimage_record_open(const char *path, int32_t *error) {
    if (strncmp(path, "IOService:", 10) != 0) {
        *error = kIOReturnBadArgument;
        return NULL;
    }
    return open_optical(path, error, true, true);
}

void discimage_probe_close(void *handle) {
    struct Probe *probe = handle;
    if (probe->exclusive) (*probe->device)->ReleaseExclusiveAccess(probe->device);
    release_probe(probe);
}

// The only data direction is target to host. The allowlist also guards this FFI
// boundary if a future caller constructs an unexpected command.
static bool allowed(const uint8_t *cdb, uint8_t length, uint32_t capacity) {
    switch (cdb[0]) {
        case 0x12:
            return length == 6 && cdb[3] == 0 && cdb[5] == 0 &&
                ((cdb[1] == 0 && cdb[2] == 0 && cdb[4] == 36 && capacity == 36) ||
                 (cdb[1] == 1 && cdb[2] == 0x80 && cdb[4] == 255 && capacity == 255));
        case 0x00: return length == 6 && capacity == 0;
        case 0x25: case 0x43: case 0x46: case 0x51: case 0x52:
            return length == 10 && capacity <= 2048;
        case 0x28: case 0xa8: {
            bool read12 = cdb[0] == 0xa8;
            if (length != (read12 ? 12 : 10)) return false;
            uint8_t count = cdb[read12 ? 9 : 8];
            if ((cdb[1] & ~8) || cdb[6] || cdb[7] || (read12 && cdb[8])) return false;
            if (count == 0) return read12 && cdb[1] == 8 && capacity == 0;
            return count <= 32 && capacity == (uint32_t)count * 2048;
        }
        case 0xbb: {
            static const uint8_t speed[] = {0xbb, 0, 0x05, 0x83, 0xff, 0xff, 0, 0, 0, 0, 0, 0};
            return length == sizeof(speed) && capacity == 0 && memcmp(cdb, speed, sizeof(speed)) == 0;
        }
        default: return false;
    }
}

static int32_t execute(struct Probe *probe, uint8_t *cdb, uint8_t length, uint8_t *data,
                       uint32_t capacity, bool write, uint32_t timeout, uint32_t *used,
                       uint8_t *status, uint8_t sense[32], uint64_t times[2]) {
    SCSITaskInterface **task = (*probe->device)->CreateSCSITask(probe->device);
    if (!task) return kIOReturnNoMemory;
    IOReturn rc = (*task)->SetCommandDescriptorBlock(task, cdb, length);
    SCSITaskSGElement buffer = { .address = (IOVirtualAddress)data, .length = capacity };
    if (rc == kIOReturnSuccess && capacity)
        rc = (*task)->SetScatterGatherEntries(task, &buffer, 1, capacity, write ? kSCSIDataTransfer_FromInitiatorToTarget : kSCSIDataTransfer_FromTargetToInitiator);
    if (rc == kIOReturnSuccess) rc = (*task)->SetTimeoutDuration(task, timeout * 1000);
    if (rc == kIOReturnSuccess) {
        SCSI_Sense_Data sense_data = {0};
        SCSITaskStatus task_status = 0;
        UInt64 transferred = 0;
        times[0] = clock_gettime_nsec_np(CLOCK_UPTIME_RAW);
        rc = (*task)->ExecuteTaskSync(task, &sense_data, &task_status, &transferred);
        times[1] = clock_gettime_nsec_np(CLOCK_UPTIME_RAW);
        if (transferred > capacity) rc = kIOReturnOverrun;
        *used = (uint32_t)transferred;
        *status = (uint8_t)task_status;
        memcpy(sense, &sense_data, sizeof(sense_data) < 32 ? sizeof(sense_data) : 32);
    }
    (*task)->Release(task);
    return rc;
}

int32_t discimage_probe_read(void *handle, uint8_t *cdb, uint8_t length, uint8_t *data,
                             uint32_t capacity, uint32_t *used, uint8_t *status,
                             uint8_t sense[32], uint64_t times[2]) {
    if (length == 0 || !allowed(cdb, length, capacity)) return kIOReturnBadArgument;
    struct Probe *probe = handle;
    if (probe->recording) return kIOReturnNotPermitted;
    if (!probe->exclusive) {
        SCSI_Sense_Data sense_data = {0};
        SCSITaskStatus task_status = 0;
        IOReturn rc;
        times[0] = clock_gettime_nsec_np(CLOCK_UPTIME_RAW);
        if (cdb[0] == 0x12 && cdb[1] == 0) {
            rc = (*probe->mmc)->Inquiry(probe->mmc, (SCSICmd_INQUIRY_StandardData *)data,
                                        capacity, &task_status, &sense_data);
        } else if (cdb[0] == 0x46 && cdb[1] == 2 && cdb[2] == 1 && cdb[3] == 8 &&
                   cdb[4] == 0 && cdb[5] == 0 && cdb[6] == 0 && cdb[7] == 2 &&
                   cdb[8] == 0 && cdb[9] == 0 && capacity == 512) {
            rc = (*probe->mmc)->GetConfiguration(probe->mmc, 2, 0x0108, data,
                                                  (SCSICmdField2Byte)capacity, &task_status, &sense_data);
        } else return kIOReturnNotPermitted;
        times[1] = clock_gettime_nsec_np(CLOCK_UPTIME_RAW);
        *status = (uint8_t)task_status;
        *used = rc == kIOReturnSuccess && task_status == kSCSITaskStatus_GOOD ? capacity : 0;
        memcpy(sense, &sense_data, sizeof(sense_data) < 32 ? sizeof(sense_data) : 32);
        return rc;
    }
    return execute(probe, cdb, length, data, capacity, false, 5, used, status, sense, times);
}



// Only the reserved recording interface may send this bounded set of commands.
static bool recording_command(const uint8_t *cdb, uint8_t length, uint32_t capacity, bool write) {
    uint32_t blocks;
    if (write) {
        switch (cdb[0]) {
            case 0x1e: return length == 6 && capacity == 0 && cdb[4] <= 1;
            case 0x35: case 0x5b: return length == 10 && capacity == 0;
            case 0x55: return length == 10 && capacity >= 60 && capacity <= 256;
            case 0x5d: return length == 10 && capacity >= 24 && capacity <= 1600 && capacity % 8 == 0;
            case 0xa1: case 0xbb: return length == 12 && capacity == 0;
            case 0x2a:
                if (length != 10) return false;
                blocks = (uint32_t)cdb[7] * 256 + cdb[8];
                return blocks > 0 && blocks <= 24 &&
                    (capacity == blocks * 2352 || capacity == blocks * 2048 || capacity == blocks * 96);
            default: return false;
        }
    }
    switch (cdb[0]) {
        case 0x00: case 0x12: return allowed(cdb, length, capacity);
        case 0x43: case 0x46: case 0x51: case 0x52: case 0x5a:
            return length == 10 && capacity > 0 && capacity <= 4096;
        case 0xac: return length == 12 && capacity >= 8 && capacity <= 2056;
        case 0x28:
            return length == 10 && cdb[7] == 0 && cdb[8] > 0 && cdb[8] <= 24 &&
                capacity == (uint32_t)cdb[8] * 2048;
        case 0xbe:
            if (length != 12 || cdb[1] != 4 || cdb[6] || cdb[7] || cdb[9] != 0x10) return false;
            blocks = cdb[8];
            return blocks > 0 && blocks <= 24 &&
                ((cdb[10] == 0 && capacity == blocks * 2352) ||
                 (cdb[10] == 1 && capacity == blocks * 2448) ||
                 (cdb[10] == 2 && capacity == blocks * 2368));
        default: return false;
    }
}

int32_t discimage_record_command(void *handle, uint8_t *cdb, uint8_t length, uint8_t *data,
                                 uint32_t capacity, bool write, uint32_t timeout, uint32_t *used,
                                 uint8_t *status, uint8_t sense[32], uint64_t times[2]) {
    struct Probe *probe = handle;
    if (!probe || !probe->exclusive || !probe->recording) return kIOReturnNotPermitted;
    if (!length || !timeout || timeout > 120 || !recording_command(cdb, length, capacity, write))
        return kIOReturnBadArgument;
    return execute(probe, cdb, length, data, capacity, write, timeout, used, status, sense, times);
}
