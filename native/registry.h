// SPDX-License-Identifier: GPL-3.0-or-later
#ifndef DISCIMAGE_REGISTRY_H
#define DISCIMAGE_REGISTRY_H

#include <IOKit/IOKitLib.h>
#include <stdbool.h>

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
