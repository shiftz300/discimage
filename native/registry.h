// SPDX-License-Identifier: GPL-3.0-or-later
#ifndef DISCIMAGE_REGISTRY_H
#define DISCIMAGE_REGISTRY_H

#include <IOKit/IOKitLib.h>
#include <stdbool.h>

static const char *const optical_classes[] = {
    "IOCompactDiscServices", "IODVDServices", "IOBDServices", NULL
};

static inline bool is_optical_service(io_registry_entry_t entry, const void *context) {
    (void)context;
    for (const char *const *type = optical_classes; *type; ++type) {
        if (IOObjectConformsTo(entry, *type)) return true;
    }
    return false;
}

// The returned service owns a reference and must still be checked for media changes.
static inline io_service_t optical_service_at_path(const char *path) {
    io_service_t service = IORegistryEntryFromPath(kIOMainPortDefault, path);
    if (service && is_optical_service(service, NULL)) return service;
    if (service) IOObjectRelease(service);
    return IO_OBJECT_NULL;
}

static inline bool whole_media(io_registry_entry_t entry) {
    CFTypeRef whole = IORegistryEntryCreateCFProperty(entry, CFSTR("Whole"), kCFAllocatorDefault, 0);
    bool result = whole && CFEqual(whole, kCFBooleanTrue);
    if (whole) CFRelease(whole);
    return result;
}

// The caller keeps its entry; a matching parent is returned with an owned reference.
static inline io_registry_entry_t matching_parent(io_registry_entry_t entry,
    bool (*matches)(io_registry_entry_t, const void *), const void *context) {
    io_registry_entry_t parent;
    if (IORegistryEntryGetParentEntry(entry, kIOServicePlane, &parent) != KERN_SUCCESS)
        return IO_OBJECT_NULL;
    for (;;) {
        if (matches(parent, context)) return parent;
        io_registry_entry_t next;
        kern_return_t status = IORegistryEntryGetParentEntry(parent, kIOServicePlane, &next);
        IOObjectRelease(parent);
        if (status != KERN_SUCCESS) return IO_OBJECT_NULL;
        parent = next;
    }
}

#endif
