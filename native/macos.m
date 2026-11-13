// SPDX-License-Identifier: GPL-3.0-or-later
// DiscRecording supplies discovery metadata and BSD media nodes.
#import <Foundation/Foundation.h>
#import <DiscRecording/DiscRecording.h>
#include <IOKit/scsi/SCSICmds_INQUIRY_Definitions.h>
#include "registry.h"

static void Require(BOOL ok, NSString *message) {
    if (!ok) @throw [NSException exceptionWithName:@"Discimage" reason:message userInfo:nil];
}
static id Number(id value) {
    if ([value isKindOfClass:[NSNumber class]]) return value;
    if ([value isKindOfClass:[DRMSF class]]) return @([(DRMSF *)value sectors]);
    return [NSNull null];
}
static NSDictionary *Media(NSDictionary *status, NSMutableArray *tracks) {
    id state=status[DRDeviceMediaStateKey];
    if (state && ![state isEqual:DRDeviceMediaStateMediaPresent]) return nil;
    NSDictionary *media=status[DRDeviceMediaInfoKey]?:status;
    if (!media[DRDeviceMediaTypeKey]) return nil;
    NSMutableSet *addresses=[NSMutableSet set];
    NSNumber *available=nil;
    // Some DiscRecording versions place track metadata inside the media dictionary.
    NSDictionary *details=media[DRDeviceTrackInfoKey]?:status[DRDeviceTrackInfoKey];
    for (id track in media[DRDeviceTrackRefsKey]?:status[DRDeviceTrackRefsKey]) {
        NSMutableDictionary *p=[[track properties] mutableCopy];
        if (!p) p=[NSMutableDictionary dictionary];
        if (details[track]) [p addEntriesFromDictionary:details[track]];
        BOOL empty=[p[DRTrackIsEmptyKey] boolValue] || [p[DRTrackTypeKey] isEqual:DRTrackTypeInvisible];
        BOOL closed=[p[DRTrackTypeKey] isEqual:DRTrackTypeClosed];
        id next=Number(p[DRNextWritableAddressKey]);
        if ([next isKindOfClass:[NSNumber class]] &&
            ([next longLongValue]<0 || [next longLongValue]>UINT32_MAX)) next=[NSNull null];
        if (!closed && (empty || [p[DRTrackTypeKey] isEqual:DRTrackTypeInvisible]) &&
            next!=[NSNull null]) {
            [addresses addObject:next];
            id free=Number(p[DRFreeBlocksKey]);
            if ([free isKindOfClass:[NSNumber class]] && (!available || [free unsignedLongLongValue]<available.unsignedLongLongValue)) available=free;
        }
        NSNumber *type=p[DRBlockTypeKey], *mode=p[DRTrackModeKey];
        BOOL audio=type ? type.intValue==kDRBlockTypeAudio : mode && (mode.intValue & 4)==0;
        [tracks addObject:@{@"number":p[DRTrackNumberKey]?:@0,@"session":p[DRSessionNumberKey]?:@0,
            @"start":Number(p[DRTrackStartAddressKey]),@"sectors":Number(p[DRTrackLengthKey]),
            @"audio":@(audio),
            @"closed":@(closed),@"empty":@(empty),@"next_writable":next}];
    }
    id free=Number(media[DRDeviceMediaBlocksFreeKey]);
    if ([free isKindOfClass:[NSNumber class]] && (!available || [free unsignedLongLongValue]<available.unsignedLongLongValue)) available=free;
    id next=addresses.count==1 ? addresses.anyObject : [NSNull null];
    NSString *kind=media[DRDeviceMediaTypeKey]?:@"unknown";
    if ([kind isEqual:DRDeviceMediaTypeCDR]) kind=@"cd-r";
    else if ([kind isEqual:DRDeviceMediaTypeCDRW]) kind=@"cd-rw";
    else if ([kind isEqual:DRDeviceMediaTypeCDROM]) kind=@"cd-rom";
    return @{@"kind":kind,@"blank":@([media[DRDeviceMediaIsBlankKey] boolValue]),
        @"appendable":@([media[DRDeviceMediaIsAppendableKey] boolValue]),
        @"reserved":@([media[DRDeviceMediaIsReservedKey] boolValue]),
        @"free_sectors":available?:[NSNull null],@"nwa":next,
        @"sessions":media[DRDeviceMediaSessionCountKey]?:@0,
        @"tracks":media[DRDeviceMediaTrackCountKey]?:@0};
}
// Use only the SCSI unit serial (VPD 80h). A USB bridge's iSerialNumber is a
// different namespace and must never masquerade as the optical drive serial.
static id HardwareIdentity(NSDictionary *info) {
    NSString *path=info[DRDeviceIORegistryEntryPathKey];
    if (!path.length) return [NSNull null];
    io_registry_entry_t entry=IORegistryEntryFromPath(kIOMainPortDefault,path.UTF8String);
    NSString *serial=nil;
    while (entry) {
        CFMutableDictionaryRef raw=NULL;
        if (IORegistryEntryCreateCFProperties(entry,&raw,kCFAllocatorDefault,0)==KERN_SUCCESS && raw) {
            NSDictionary *properties=CFBridgingRelease(raw);
            id value=properties[@kIOPropertySCSIINQUIRYUnitSerialNumber];
            if (!value) value=properties[@"SCSI Device Characteristics"][@kIOPropertySCSIINQUIRYUnitSerialNumber];
            if ([value isKindOfClass:[NSString class]]) serial=value;
        }
        io_registry_entry_t parent=IO_OBJECT_NULL;
        if (!serial) IORegistryEntryGetParentEntry(entry,kIOServicePlane,&parent);
        IOObjectRelease(entry);
        entry=parent;
        if (serial) break;
    }
    if (!serial.length) return [NSNull null];
    return @{@"vendor":info[DRDeviceVendorNameKey]?:@"",@"product":info[DRDeviceProductNameKey]?:@"",@"serial":serial};
}

static NSDictionary *Snapshot(DRDevice *device) {
    NSDictionary *info=[device info], *caps=info[DRDeviceWriteCapabilitiesKey];
    NSMutableArray *tracks=[NSMutableArray array];
    id state=Media([device status],tracks)?:[NSNull null];
    return @{@"hardware":HardwareIdentity(info),@"id":info[DRDeviceIORegistryEntryPathKey]?:@"",
        @"name":[NSString stringWithFormat:@"%@ %@ (%@)",info[DRDeviceVendorNameKey]?:@"",info[DRDeviceProductNameKey]?:@"",info[DRDeviceFirmwareRevisionKey]?:@""],
        @"backend":@"macos-iokit-mmc",
        @"capabilities":@{@"cdr":@([caps[DRDeviceCanWriteCDRKey] boolValue]),
            @"cdrw":@([caps[DRDeviceCanWriteCDRWKey] boolValue]),
            @"sao":@([caps[DRDeviceCanWriteCDSAOKey] boolValue]),@"tao":@([caps[DRDeviceCanWriteCDTAOKey] boolValue]),
            @"cd_text":@([caps[DRDeviceCanWriteCDTextKey] boolValue])},
        @"media":state,@"tracks":tracks};
}
static DRDevice *Find(NSString *identifier) {
    for (DRDevice *device in [DRDevice devices]) {
        if ([[device info][DRDeviceIORegistryEntryPathKey] isEqual:identifier]) return device;
    }
    Require(NO,@"Selected optical drive is no longer connected"); return nil;
}
static void Pump(double seconds) {
    [[NSRunLoop currentRunLoop] runUntilDate:[NSDate dateWithTimeIntervalSinceNow:seconds]];
}
static NSDictionary *ReadySnapshot(DRDevice *device, int (*cancelled)(void)) {
    NSDate *deadline=[NSDate dateWithTimeIntervalSinceNow:15];
    NSDictionary *snapshot;
    do {
        Pump(0.2);
        snapshot=Snapshot(device);
    } while (snapshot[@"media"]==[NSNull null] && device.isValid &&
             deadline.timeIntervalSinceNow>0 && (!cancelled || !cancelled()));
    return snapshot;
}

static bool DrivePathMatches(io_registry_entry_t entry, const void *drivePath) {
    io_string_t path;
    return IORegistryEntryGetPath(entry,kIOServicePlane,path)==KERN_SUCCESS && strcmp(path,drivePath)==0;
}

static NSDictionary *OpticalMedium(NSString *path) {
    if (path.length==0) return nil;
    io_iterator_t media;
    if (IOServiceGetMatchingServices(kIOMainPortDefault,IOServiceMatching("IOCDMedia"),&media)!=KERN_SUCCESS) return nil;
    NSDictionary *result=nil;
    io_service_t item;
    while ((item=IOIteratorNext(media))) {
        uint64_t identifier;
        io_registry_entry_t drive=whole_media(item) ? matching_parent(item,DrivePathMatches,path.UTF8String) : IO_OBJECT_NULL;
        BOOL found=drive!=IO_OBJECT_NULL &&
            IORegistryEntryGetRegistryEntryID(item,&identifier)==KERN_SUCCESS;
        if (drive) IOObjectRelease(drive);
        if (found) {
            if (result) {
                IOObjectRelease(item);
                result=nil;
                break;
            }
            id node=CFBridgingRelease(IORegistryEntryCreateCFProperty(item,CFSTR("BSD Name"),kCFAllocatorDefault,0));
            result=@{@"media_id":@(identifier),@"bsd_name":[node isKindOfClass:[NSString class]]?node:[NSNull null]};
        }
        IOObjectRelease(item);
    }
    IOObjectRelease(media);
    return result;
}

// Read-only bridge. Recording and timing use separate IOKit task handles.
char *discimage_macos_query(const char *json, int (*cancelled)(void)) {
    @autoreleasepool { @try {
        NSData *bytes=[[NSString stringWithUTF8String:json] dataUsingEncoding:NSUTF8StringEncoding];
        NSDictionary *request=[NSJSONSerialization JSONObjectWithData:bytes options:0 error:NULL];
        Require([request isKindOfClass:[NSDictionary class]],@"Invalid native request");
        NSString *op=request[@"op"];
        NSDictionary *result=nil;
        if ([op isEqual:@"list"]) {
            NSMutableArray *devices=[NSMutableArray array];
            for (DRDevice *device in [DRDevice devices]) [devices addObject:Snapshot(device)];
            result=@{@"devices":devices};
        } else {
            NSString *path=request[@"device"];
            DRDevice *device=Find(path);
            if ([op isEqual:@"probe"]) result=Snapshot(device);
            else if ([op isEqual:@"media_node"] || [op isEqual:@"probe_node"]) {
                BOOL timing=[op isEqual:@"probe_node"];
                NSDictionary *snapshot=timing ? ReadySnapshot(device,cancelled) : Snapshot(device);
                id media=snapshot[@"media"];
                Require([media isKindOfClass:[NSDictionary class]],@"No ready optical medium");
                if (timing) {
                    NSArray *tracks=snapshot[@"tracks"];
                    Require(![media[@"blank"] boolValue] && ![media[@"appendable"] boolValue] &&
                        [media[@"sessions"] unsignedIntValue]==1 && tracks.count==1 &&
                        ![tracks.firstObject[@"audio"] boolValue],
                        @"Expected the finalized single-data-track target before unmounting");
                }
                result=OpticalMedium(path);
                Require(result!=nil && (!timing || [result[@"bsd_name"] isKindOfClass:[NSString class]]),
                    @"Cannot identify the selected optical medium");
            } else Require(NO,@"Unknown optical metadata request");
        }
        NSData *output=[NSJSONSerialization dataWithJSONObject:result options:0 error:NULL];
        Require(output!=nil,@"Could not encode optical metadata");
        NSString *response=[[NSString alloc] initWithData:output encoding:NSUTF8StringEncoding];
        return strdup(response.UTF8String);
    } @catch (NSException *exception) {
        NSData *output=[NSJSONSerialization dataWithJSONObject:@{@"error":exception.reason?:@"Native optical error"}
            options:0 error:NULL];
        NSString *response=[[NSString alloc] initWithData:output encoding:NSUTF8StringEncoding];
        return strdup(response.UTF8String);
    } }
}

void discimage_free(char *value) { free(value); }
