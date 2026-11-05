// SPDX-License-Identifier: GPL-3.0-or-later
// Narrow DiscRecording bridge. Rust owns policy, authoring, hashes and journaling.
#import <Foundation/Foundation.h>
#import <DiscRecording/DiscRecording.h>
#include <unistd.h>
#include <sys/stat.h>

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
    return @{@"kind":kind,@"blank":@([media[DRDeviceMediaIsBlankKey] boolValue]),
        @"appendable":@([media[DRDeviceMediaIsAppendableKey] boolValue]),
        @"reserved":@([media[DRDeviceMediaIsReservedKey] boolValue]),
        @"free_sectors":available?:[NSNull null],@"nwa":next,
        @"sessions":media[DRDeviceMediaSessionCountKey]?:@0,
        @"tracks":media[DRDeviceMediaTrackCountKey]?:@0};
}
static NSDictionary *Snapshot(DRDevice *device) {
    NSDictionary *info=[device info], *caps=info[DRDeviceWriteCapabilitiesKey];
    NSMutableArray *tracks=[NSMutableArray array];
    id state=Media([device status],tracks)?:[NSNull null];
    return @{@"id":info[DRDeviceIORegistryEntryPathKey]?:@"",
        @"name":[NSString stringWithFormat:@"%@ %@ (%@)",info[DRDeviceVendorNameKey]?:@"",info[DRDeviceProductNameKey]?:@"",info[DRDeviceFirmwareRevisionKey]?:@""],
        @"backend":@"macos-discrecording",
        @"capabilities":@{@"cdr":@([caps[DRDeviceCanWriteCDRKey] boolValue]),
            @"cdrw":@([caps[DRDeviceCanWriteCDRWKey] boolValue]),
            @"sao":@([caps[DRDeviceCanWriteCDSAOKey] boolValue]),@"tao":@([caps[DRDeviceCanWriteCDTAOKey] boolValue])},
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

static NSDictionary *ReadbackMedia(DRDevice *device, int (*cancelled)(void)) {
    NSDate *deadline=[NSDate dateWithTimeIntervalSinceNow:10];
    do {
        Require(device.isValid,@"Optical drive disconnected");
        Require(!cancelled || !cancelled(),@"Audio readback cancelled");
        NSDictionary *status=[device status];
        NSDictionary *media=status[DRDeviceMediaInfoKey]?:status;
        if ([status[DRDeviceMediaStateKey] isEqual:DRDeviceMediaStateMediaPresent]) {
            Require([media[DRDeviceMediaIsReservedKey] boolValue],@"CD reservation was lost");
            NSString *node=media[DRDeviceMediaBSDNameKey];
            if (node.length>0) return @{@"bsd_name":node};
        }
        Pump(0.2);
    } while (deadline.timeIntervalSinceNow>0);
    Require(NO,@"macOS has not exposed a raw device for the recorded CD");
    return nil;
}

@interface DISession : NSObject
@property(nonatomic,strong) DRDevice *device;
@property(nonatomic,assign) BOOL reservation;
@end
@implementation DISession
- (void)dealloc { if (_reservation) [_device releaseMediaReservation]; }
@end

static DRTrack *DataTrack(NSString *path, uint32_t sectors) {
    struct stat status;
    Require(lstat(path.fileSystemRepresentation,&status)==0 && S_ISREG(status.st_mode) &&
        (uint64_t)status.st_size==(uint64_t)sectors*2048,@"Data ISO is missing or has the wrong length");
    id layout=[DRBurn layoutForImageFile:path];
    Require([layout isKindOfClass:[DRTrack class]],@"DiscRecording could not open the ISO as a single data track");
    DRTrack *track=layout;
    Require([Number(track.properties[DRTrackLengthKey]) isEqual:@(sectors)],
        @"DiscRecording changed the planned ISO length");
    return track;
}
static DRTrack *OpenAudio(NSString *path) {
    DRTrack *track=[DRTrack trackForAudioFile:path];
    Require(track!=nil,@"DiscRecording could not open the normalized WAV");
    return track;
}
static DRTrack *AudioTrack(NSDictionary *input, BOOL first) {
    DRTrack *track=OpenAudio(input[@"path"]);
    NSMutableDictionary *properties=[[track properties] mutableCopy];
    NSNumber *expected=input[@"sectors"];
    id reported=Number(properties[DRTrackLengthKey]);
    NSString *path=input[@"path"];
    Require([reported isEqual:expected],
        [NSString stringWithFormat:@"System audio decoder changed the planned track length for %@ (%@ vs %@ sectors)",
            path.lastPathComponent,reported,expected]);
    properties[DRPreGapLengthKey]=first?@150:input[@"gap"];
    properties[DRPreGapIsRequiredKey]=@YES;
    properties[DRVerificationTypeKey]=DRVerificationTypeNone;
    [track setProperties:properties];
    return track;
}
static int StatusPercent(id value) {
    if (![value isKindOfClass:[NSNumber class]]) return -1;
    double fraction=[value doubleValue];
    return fraction>=0 && fraction<=1 ? (int)(fraction*100) : -1;
}
static NSDictionary *Write(DISession *session, NSDictionary *request, int (*cancelled)(void)) {
    DRDevice *device=session.device;
    Require(device.isValid,@"Optical drive disconnected");
    NSDictionary *before=Snapshot(device),*media=before[@"media"],*caps=before[@"capabilities"];
    Require([media isKindOfClass:[NSDictionary class]],@"A reserved, writable CD is required");
    NSString *kind=media[@"kind"];
    BOOL writable=([kind isEqual:@"cd-r"] && [caps[@"cdr"] boolValue]) ||
        ([kind isEqual:@"cd-rw"] && [caps[@"cdrw"] boolValue]);
    Require(writable && [media[@"reserved"] boolValue],@"A reserved, writable CD-R or CD-RW is required");
    BOOL audio=[request[@"op"] isEqual:@"audio"];
    NSMutableArray *tracks=[NSMutableArray array];
    if (audio) {
        Require([media[@"blank"] boolValue] && [caps[@"sao"] boolValue],@"Audio requires a blank writable CD and SAO support");
        NSArray *inputs=request[@"tracks"];
        Require(inputs.count>=1 && inputs.count<=98,@"Invalid music track count");
        for (NSDictionary *input in inputs) [tracks addObject:AudioTrack(input,tracks.count==0)];
    } else {
        Require(![media[@"blank"] boolValue] && [media[@"appendable"] boolValue] && [caps[@"tao"] boolValue],@"Data append requires an appendable CD and TAO support");
        NSNumber *start=request[@"start"],*count=request[@"sectors"];
        Require([media[@"nwa"] isEqual:start] && [media[@"free_sectors"] isKindOfClass:[NSNumber class]] &&
            [media[@"free_sectors"] unsignedLongLongValue]>=count.unsignedLongLongValue,@"NWA changed or remaining capacity is insufficient");
        [tracks addObject:DataTrack(request[@"path"],count.unsignedIntValue)];
    }
    double speed=[request[@"speed"] doubleValue]*DRDeviceBurnSpeedCD1x;
    Require(speed>0,@"Recording speed must be positive");
    Require(!cancelled || !cancelled(),@"Recording cancelled before write");
    DRBurn *burn=[DRBurn burnForDevice:device];
    [burn setProperties:@{DRBurnAppendableKey:@(audio),DRBurnOverwriteDiscKey:@NO,
        DRBurnVerifyDiscKey:@(!audio),DRBurnTestingKey:@NO,DRBurnUnderrunProtectionKey:@YES,
        DRBurnRequestedSpeedKey:@(speed),DRBurnStrategyKey:audio?DRBurnStrategyCDSAO:DRBurnStrategyCDTAO,
        DRBurnStrategyIsRequiredKey:@YES,DRBurnCompletionActionKey:DRBurnCompletionActionMount,
        DRBurnFailureActionKey:DRBurnFailureActionNone}];
    [burn writeLayout:tracks];
    BOOL aborting=NO;
    NSString *previous=@"";
    int previousBucket=-2, highPercent=-1;
    double started=NSProcessInfo.processInfo.systemUptime, lastReport=started, lastChange=started;
    for (;;) { @autoreleasepool {
        NSDictionary *status=[burn status]; NSString *state=status[DRStatusStateKey]?:@"unknown";
        double now=NSProcessInfo.processInfo.systemUptime;
        int percent=StatusPercent(status[DRStatusPercentCompleteKey]);
        int bucket=percent<0 ? -1 : percent/5;
        BOOL changed=![state isEqual:previous];
        if (changed || percent>highPercent) { highPercent=percent; lastChange=now; }
        if (changed || bucket!=previousBucket || now-lastReport>=15) {
            NSString *prefix=@"DRStatusState";
            NSString *label=[state hasPrefix:prefix] ? [state substringFromIndex:prefix.length] : state;
            NSNumber *track=status[DRStatusCurrentTrackKey];
            NSNumber *rate=status[DRStatusProgressInfoKey][DRStatusProgressCurrentXFactor];
            fprintf(stderr,"%s: %s",audio?"Music":"Data",label.UTF8String);
            if (track.intValue>0) fprintf(stderr," · track %d",track.intValue);
            if (percent>=0) fprintf(stderr," · system %d%%",percent);
            else fprintf(stderr," · percent unavailable");
            if (rate.doubleValue>0) fprintf(stderr," · %.1fx",rate.doubleValue);
            fprintf(stderr," · %lus elapsed",(unsigned long)(now-started));
            if (now-lastChange>=15) fprintf(stderr," · unchanged for %lus",(unsigned long)(now-lastChange));
            fputc('\n',stderr);
            previous=[state copy]; previousBucket=bucket; lastReport=now;
        }
        if ([state isEqual:DRStatusStateFailed]) {
            Require(NO,[NSString stringWithFormat:@"DiscRecording failed: %@",status[DRErrorStatusKey]?:status]);
        }
        if ([state isEqual:DRStatusStateDone]) {
            Require(!aborting,@"Recording aborted; inspect media before any further action"); break;
        }
        if (!aborting && cancelled && cancelled()) { [burn abort];aborting=YES; }
        Pump(0.1);usleep(100000);
    }}
    Pump(1.0);
    return @{ @"ok":@YES };
}

// Every result is owned by Rust and released with discimage_free.
char *discimage_call(void **handle, const char *json, int (*cancelled)(void)) {
    @autoreleasepool { @try {
        NSData *bytes=[[NSString stringWithUTF8String:json] dataUsingEncoding:NSUTF8StringEncoding];
        NSDictionary *request=[NSJSONSerialization JSONObjectWithData:bytes options:0 error:NULL];
        Require([request isKindOfClass:[NSDictionary class]],@"Invalid native request");
        NSString *op=request[@"op"]; NSDictionary *result=nil;
        if ([op isEqual:@"list"]) {
            NSMutableArray *devices=[NSMutableArray array];
            for (DRDevice *device in [DRDevice devices]) [devices addObject:Snapshot(device)];
            result=@{@"devices":devices};
        } else if ([op isEqual:@"probe"]) {
            result=Snapshot(Find(request[@"device"]));
        } else if ([op isEqual:@"audio_length"]) {
            DRTrack *track=OpenAudio(request[@"path"]);
            result=@{@"sectors":Number(track.properties[DRTrackLengthKey])};
        } else if ([op isEqual:@"open"]) {
            Require(*handle==NULL,@"Recorder already open");
            DISession *session=[DISession new];session.device=Find(request[@"device"]);
            Require([Snapshot(session.device)[@"media"] isKindOfClass:[NSDictionary class]],@"No ready media in drive");
            [session.device acquireMediaReservation];session.reservation=YES;
            NSDate *deadline=[NSDate dateWithTimeIntervalSinceNow:5];
            while (![Snapshot(session.device)[@"media"][@"reserved"] boolValue] && deadline.timeIntervalSinceNow>0) Pump(0.1);
            Require([Snapshot(session.device)[@"media"][@"reserved"] boolValue],@"Media is reserved by another application");
            *handle=(__bridge_retained void *)session;result=@{@"ok":@YES};
        } else if ([op isEqual:@"close"]) {
            if (*handle) { (void)CFBridgingRelease(*handle);*handle=NULL; }
            result=@{@"ok":@YES};
        } else if ([op isEqual:@"selftest"]) {
            DRTrack *track=DataTrack(request[@"path"],4);
            DRTrack *audio=AudioTrack(@{@"path":request[@"audio"],@"sectors":request[@"audio_sectors"],@"gap":@0},YES);
            const void *keys[]={(__bridge const void *)audio,(__bridge const void *)track};
            NSDictionary *closed=@{DRBlockTypeKey:@(kDRBlockTypeAudio),DRTrackTypeKey:DRTrackTypeClosed,
                DRNextWritableAddressKey:@(-145)};
            NSDictionary *invisible=@{DRTrackTypeKey:DRTrackTypeInvisible,DRNextWritableAddressKey:@11850};
            const void *values[]={(__bridge const void *)closed,(__bridge const void *)invisible};
            NSDictionary *details=CFBridgingRelease(CFDictionaryCreate(NULL,keys,values,2,
                &kCFTypeDictionaryKeyCallBacks,&kCFTypeDictionaryValueCallBacks));
            NSDictionary *media=@{DRDeviceMediaTypeKey:DRDeviceMediaTypeCDRW,DRDeviceTrackRefsKey:@[audio,track],
                DRDeviceTrackInfoKey:details};
            NSMutableArray *tracks=[NSMutableArray array];
            NSDictionary *parsed=Media(@{DRDeviceMediaStateKey:DRDeviceMediaStateMediaPresent,
                DRDeviceMediaInfoKey:media},tracks);
            result=@{@"audio_sectors":Number(audio.properties[DRTrackLengthKey]),@"image_sectors":@(track.estimateLength),
                @"image_produces":@((BOOL)([track testProductionSpeedForLength:4*2048]>0)),
                @"nwa":parsed[@"nwa"],@"audio_track":tracks.firstObject[@"audio"],
                @"closed_track_nwa":tracks.firstObject[@"next_writable"],
                @"status_percent":@[@(StatusPercent(nil)),@(StatusPercent(@(-1))),@(StatusPercent(@0.375))]};
        } else {
            Require(*handle!=NULL,@"Recorder is not reserved");
            DISession *session=(__bridge DISession *)*handle;
            Require(session.device.isValid,@"Optical drive disconnected");
            if ([op isEqual:@"status"]) result=ReadySnapshot(session.device,cancelled);
            else if ([op isEqual:@"media_node"]) result=ReadbackMedia(session.device,cancelled);
            else { Require([op isEqual:@"audio"] || [op isEqual:@"data"],@"Unknown recorder operation");result=Write(session,request,cancelled); }
        }
        NSData *data=[NSJSONSerialization dataWithJSONObject:result options:0 error:NULL];
        Require(data!=nil,@"Native response serialization failed");
        return strdup([[NSString alloc] initWithData:data encoding:NSUTF8StringEncoding].UTF8String);
    } @catch (NSException *exception) {
        NSData *data=[NSJSONSerialization dataWithJSONObject:@{@"error":exception.reason?:@"Native recording exception"} options:0 error:NULL];
        return strdup([[NSString alloc] initWithData:data encoding:NSUTF8StringEncoding].UTF8String);
    }}
}
void discimage_free(char *value) { free(value); }
