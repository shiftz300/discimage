// SPDX-License-Identifier: GPL-3.0-or-later
// Narrow DiscRecording bridge. Rust owns policy, authoring, hashes and journaling.
#import <Foundation/Foundation.h>
#import <DiscRecording/DiscRecording.h>
#include <fcntl.h>
#include <unistd.h>
#include <errno.h>
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
    if (![status[DRDeviceMediaStateKey] isEqual:DRDeviceMediaStateMediaPresent]) return nil;
    NSDictionary *media=status[DRDeviceMediaInfoKey];
    NSMutableSet *addresses=[NSMutableSet set];
    NSNumber *available=nil;
    NSDictionary *details=status[DRDeviceTrackInfoKey];
    for (id track in status[DRDeviceTrackRefsKey]) {
        NSMutableDictionary *p=[[track properties] mutableCopy];
        if (!p) p=[NSMutableDictionary dictionary];
        if (details[track]) [p addEntriesFromDictionary:details[track]];
        BOOL empty=[p[DRTrackIsEmptyKey] boolValue] || [p[DRTrackTypeKey] isEqual:DRTrackTypeInvisible];
        BOOL closed=[p[DRTrackTypeKey] isEqual:DRTrackTypeClosed];
        id next=Number(p[DRNextWritableAddressKey]);
        if (!closed && (empty || [p[DRTrackTypeKey] isEqual:DRTrackTypeInvisible]) &&
            [next isKindOfClass:[NSNumber class]] && [next longLongValue]>=0) {
            [addresses addObject:next];
            id free=Number(p[DRFreeBlocksKey]);
            if ([free isKindOfClass:[NSNumber class]] && (!available || [free unsignedLongLongValue]<available.unsignedLongLongValue)) available=free;
        }
        [tracks addObject:@{@"number":p[DRTrackNumberKey]?:@0,@"session":p[DRSessionNumberKey]?:@0,
            @"start":Number(p[DRTrackStartAddressKey]),@"sectors":Number(p[DRTrackLengthKey]),
            @"audio":@(p[DRTrackModeKey]!=nil && [p[DRTrackModeKey] intValue]==0),
            @"closed":@(closed),@"empty":@(empty),@"next_writable":next}];
    }
    id free=Number(media[DRDeviceMediaBlocksFreeKey]);
    if ([free isKindOfClass:[NSNumber class]] && (!available || [free unsignedLongLongValue]<available.unsignedLongLongValue)) available=free;
    id next=addresses.count==1 ? addresses.anyObject : [NSNull null];
    NSString *kind=[media[DRDeviceMediaTypeKey] isEqual:DRDeviceMediaTypeCDR]?@"cd-r":media[DRDeviceMediaTypeKey]?:@"unknown";
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

@interface DISession : NSObject
@property(nonatomic,strong) DRDevice *device;
@property(nonatomic,assign) BOOL reservation;
@end
@implementation DISession
- (void)dealloc { if (_reservation) [_device releaseMediaReservation]; }
@end

@interface DIProducer : NSObject
@property(nonatomic,assign) int fd;
@property(nonatomic,assign) uint64_t bytes;
@property(nonatomic,assign) BOOL guarded;
@property(nonatomic,assign) uint32_t expected;
- (instancetype)initWithPath:(NSString *)path sectors:(uint32_t)sectors;
- (uint32_t)produceDataForTrack:(DRTrack *)track intoBuffer:(char *)buffer length:(uint32_t)length atAddress:(uint64_t)address blockSize:(uint32_t)blockSize ioFlags:(uint32_t *)flags;
@end
@implementation DIProducer
- (instancetype)initWithPath:(NSString *)path sectors:(uint32_t)sectors {
    self=[super init]; if (!self) return nil;
    _fd=-1; _bytes=(uint64_t)sectors*2048;
    _fd=open(path.fileSystemRepresentation,O_RDONLY|O_NOFOLLOW);
    struct stat status;
    Require(_fd>=0 && fstat(_fd,&status)==0 && S_ISREG(status.st_mode) && (uint64_t)status.st_size==_bytes,@"Data ISO is missing, changed, or has the wrong length");
    return self;
}
- (void)dealloc { if (_fd>=0) close(_fd); }
- (uint64_t)estimateLengthOfTrack:(__unused DRTrack *)track { return _bytes/2048; }
- (BOOL)prepareTrack:(DRTrack *)track forBurn:(__unused DRBurn *)burn toMedia:(NSDictionary *)media {
    if (!_guarded) return YES; // Only used by the read-only bridge self-test.
    NSDictionary *m=Media(media,nil);
    id assigned=Number([track properties][DRTrackStartAddressKey]);
    return [m[@"reserved"] boolValue] && [m[@"appendable"] boolValue] &&
        [m[@"nwa"] isKindOfClass:[NSNumber class]] && [m[@"nwa"] unsignedIntValue]==_expected &&
        (![assigned isKindOfClass:[NSNumber class]] || [assigned unsignedIntValue]==_expected);
}
- (uint32_t)produceDataForTrack:(__unused DRTrack *)track intoBuffer:(char *)buffer length:(uint32_t)length atAddress:(uint64_t)address blockSize:(uint32_t)blockSize ioFlags:(uint32_t *)flags {
    if (blockSize!=2048 || (flags && (*flags & kDRFlagSubchannelDataRequested)) || address>_bytes/2048) return 0;
    uint64_t offset=address*2048;
    uint32_t wanted=(uint32_t)MIN((uint64_t)(length/2048*2048),_bytes-offset), done=0;
    while (done<wanted) {
        ssize_t count=pread(_fd,buffer+done,wanted-done,(off_t)(offset+done));
        if (count<0 && errno==EINTR) continue;
        if (count<=0) return 0;
        done+=(uint32_t)count;
    }
    return done;
}
- (BOOL)prepareTrackForVerification:(__unused DRTrack *)track { return YES; }
@end

static DRTrack *DataTrack(DIProducer *producer, uint32_t sectors) {
    DRTrack *track=[[DRTrack alloc] initWithProducer:producer];
    [track setProperties:@{DRTrackLengthKey:[DRMSF msfWithFrames:sectors],DRBlockSizeKey:@2048,
        DRBlockTypeKey:@8,DRDataFormKey:@16,DRTrackModeKey:@4,DRSessionFormatKey:@0,
        DRPreGapLengthKey:@150,DRPreGapIsRequiredKey:@YES,DRVerificationTypeKey:DRVerificationTypeProduceAgain}];
    return track;
}
static DRTrack *AudioTrack(NSDictionary *input, BOOL first) {
    DRTrack *track=[DRTrack trackForAudioFile:input[@"path"]];
    Require(track!=nil,@"DiscRecording could not open the normalized WAV");
    Require([Number([track properties][DRTrackLengthKey]) isEqual:input[@"sectors"]],@"System audio decoder changed the planned track length");
    NSMutableDictionary *properties=[[track properties] mutableCopy];
    properties[DRPreGapLengthKey]=first?@150:input[@"gap"];
    properties[DRPreGapIsRequiredKey]=@YES;
    properties[DRVerificationTypeKey]=DRVerificationTypeChecksum;
    [track setProperties:properties];
    return track;
}
static NSDictionary *Write(DISession *session, NSDictionary *request, int (*cancelled)(void)) {
    DRDevice *device=session.device;
    Require(device.isValid,@"Optical drive disconnected");
    NSDictionary *before=Snapshot(device),*media=before[@"media"],*caps=before[@"capabilities"];
    Require([media isKindOfClass:[NSDictionary class]] && [media[@"kind"] isEqual:@"cd-r"] &&
        [media[@"reserved"] boolValue] && [caps[@"cdr"] boolValue],@"A reserved, writable CD-R is required");
    BOOL audio=[request[@"op"] isEqual:@"audio"];
    NSMutableArray *tracks=[NSMutableArray array];
    if (audio) {
        Require([media[@"blank"] boolValue] && [caps[@"sao"] boolValue],@"Audio requires a blank CD-R and SAO support");
        NSArray *inputs=request[@"tracks"];
        Require(inputs.count>=1 && inputs.count<=98,@"Invalid music track count");
        for (NSDictionary *input in inputs) [tracks addObject:AudioTrack(input,tracks.count==0)];
    } else {
        Require(![media[@"blank"] boolValue] && [media[@"appendable"] boolValue] && [caps[@"tao"] boolValue],@"Data append requires appendable CD-R and TAO support");
        NSNumber *start=request[@"start"],*count=request[@"sectors"];
        Require([media[@"nwa"] isEqual:start] && [media[@"free_sectors"] isKindOfClass:[NSNumber class]] &&
            [media[@"free_sectors"] unsignedLongLongValue]>=count.unsignedLongLongValue,@"NWA changed or remaining capacity is insufficient");
        DIProducer *producer=[[DIProducer alloc] initWithPath:request[@"path"] sectors:count.unsignedIntValue];
        producer.guarded=YES;producer.expected=start.unsignedIntValue;
        [tracks addObject:DataTrack(producer,count.unsignedIntValue)];
    }
    double speed=[request[@"speed"] doubleValue]*DRDeviceBurnSpeedCD1x;
    Require(speed>0,@"Recording speed must be positive");
    Require(!cancelled || !cancelled(),@"Recording cancelled before write");
    DRBurn *burn=[DRBurn burnForDevice:device];
    [burn setProperties:@{DRBurnAppendableKey:@(audio),DRBurnOverwriteDiscKey:@NO,
        DRBurnVerifyDiscKey:@YES,DRBurnTestingKey:@NO,DRBurnUnderrunProtectionKey:@YES,
        DRBurnRequestedSpeedKey:@(speed),DRBurnStrategyKey:audio?DRBurnStrategyCDSAO:DRBurnStrategyCDTAO,
        DRBurnStrategyIsRequiredKey:@YES,DRBurnCompletionActionKey:DRBurnCompletionActionMount,
        DRBurnFailureActionKey:DRBurnFailureActionNone}];
    [burn writeLayout:tracks];
    BOOL aborting=NO;
    NSString *previous=@""; int previousPercent=-1;
    for (;;) { @autoreleasepool {
        NSDictionary *status=[burn status]; NSString *state=status[DRStatusStateKey]?:@"unknown";
        int percent=(int)([status[DRStatusPercentCompleteKey] doubleValue]*100);
        if (![state isEqual:previous] || percent/5!=previousPercent/5) {
            fprintf(stderr,"%s: %s %d%%\n",audio?"Music":"Data",state.UTF8String,percent);
            previous=[state copy];previousPercent=percent;
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
            DIProducer *producer=[[DIProducer alloc] initWithPath:request[@"path"] sectors:4];
            DRTrack *track=DataTrack(producer,4);char buffer[4096];uint32_t flags=0;
            uint32_t count=[producer produceDataForTrack:track intoBuffer:buffer length:sizeof buffer atAddress:1 blockSize:2048 ioFlags:&flags];
            Require(count==4096,@"Producer test returned wrong length");
            DRTrack *audio=AudioTrack(@{@"path":request[@"audio"],@"sectors":@300,@"gap":@0},YES);
            result=@{@"bytes":@(count),@"first":@((unsigned char)buffer[0]),@"last":@((unsigned char)buffer[4095]),
                @"audio_sectors":Number([audio properties][DRTrackLengthKey]),@"data_block_size":[track properties][DRBlockSizeKey]};
        } else {
            Require(*handle!=NULL,@"Recorder is not reserved");
            DISession *session=(__bridge DISession *)*handle;
            Require(session.device.isValid,@"Optical drive disconnected");
            if ([op isEqual:@"status"]) { Pump(0.2);result=Snapshot(session.device); }
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
