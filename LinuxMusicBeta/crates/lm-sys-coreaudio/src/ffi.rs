//! Hand-written declarations of the CoreAudio calls used (see the crate docs for why not `coreaudio-sys`). Every
//! constant is a four-character code of `AudioHardware.h`, `AudioHardwareBase.h` or `AudioHardwareDeprecated.h`.
#![allow(non_camel_case_types, non_upper_case_globals, non_snake_case, dead_code)]

use std::ffi::c_void;

use core_foundation_sys::base::CFTypeRef;
use core_foundation_sys::dictionary::CFDictionaryRef;
use core_foundation_sys::string::CFStringRef;

pub type OSStatus = i32;
pub type AudioObjectID = u32;
pub type AudioDeviceID = u32;
pub type AudioStreamID = u32;

pub const fn fourcc(s: &[u8; 4]) -> u32 {
    ((s[0] as u32) << 24) | ((s[1] as u32) << 16) | ((s[2] as u32) << 8) | s[3] as u32
}

pub const kAudioObjectSystemObject: AudioObjectID = 1;
pub const kAudioObjectPropertyElementMain: u32 = 0;

pub const kAudioObjectPropertyScopeGlobal: u32 = fourcc(b"glob");
pub const kAudioObjectPropertyScopeInput: u32 = fourcc(b"inpt");
pub const kAudioObjectPropertyScopeOutput: u32 = fourcc(b"outp");

pub const kAudioHardwarePropertyDevices: u32 = fourcc(b"dev#");
pub const kAudioHardwarePropertyDefaultInputDevice: u32 = fourcc(b"dIn ");
pub const kAudioHardwarePropertyDefaultOutputDevice: u32 = fourcc(b"dOut");
pub const kAudioHardwarePropertyTranslateUIDToDevice: u32 = fourcc(b"uidd");

pub const kAudioObjectPropertyName: u32 = fourcc(b"lnam");
pub const kAudioDevicePropertyDeviceUID: u32 = fourcc(b"uid ");
pub const kAudioDevicePropertyStreamConfiguration: u32 = fourcc(b"slay");
pub const kAudioDevicePropertyStreams: u32 = fourcc(b"stm#");
pub const kAudioDevicePropertyNominalSampleRate: u32 = fourcc(b"nsrt");
pub const kAudioDevicePropertyAvailableNominalSampleRates: u32 = fourcc(b"nsr#");
pub const kAudioDevicePropertyBufferFrameSize: u32 = fourcc(b"fsiz");
pub const kAudioDevicePropertyBufferFrameSizeRange: u32 = fourcc(b"fsz#");
pub const kAudioDevicePropertyLatency: u32 = fourcc(b"ltnc");
pub const kAudioDevicePropertySafetyOffset: u32 = fourcc(b"saft");
pub const kAudioDevicePropertyDeviceIsAlive: u32 = fourcc(b"livn");
pub const kAudioDevicePropertyIOProcStreamUsage: u32 = fourcc(b"suse");
pub const kAudioDeviceProcessorOverload: u32 = fourcc(b"over");
pub const kAudioStreamPropertyLatency: u32 = fourcc(b"ltnc");

pub const kAudioHardwareNoError: OSStatus = 0;
pub const kAudioHardwareNotRunningError: OSStatus = fourcc(b"stop") as OSStatus;
pub const kAudioHardwareBadObjectError: OSStatus = fourcc(b"!obj") as OSStatus;
pub const kAudioHardwareBadDeviceError: OSStatus = fourcc(b"!dev") as OSStatus;
pub const kAudioHardwareBadStreamError: OSStatus = fourcc(b"!str") as OSStatus;
pub const kAudioHardwareUnknownPropertyError: OSStatus = fourcc(b"who?") as OSStatus;
pub const kAudioHardwareIllegalOperationError: OSStatus = fourcc(b"nope") as OSStatus;
pub const kAudioHardwareUnsupportedOperationError: OSStatus = fourcc(b"unop") as OSStatus;

pub const kAudioTimeStampHostTimeValid: u32 = 1 << 1;

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct AudioObjectPropertyAddress {
    pub mSelector: u32,
    pub mScope: u32,
    pub mElement: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct AudioValueRange {
    pub mMinimum: f64,
    pub mMaximum: f64,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct AudioBuffer {
    pub mNumberChannels: u32,
    pub mDataByteSize: u32,
    pub mData: *mut c_void,
}

/// A list of buffers: `mBuffers` really has `mNumberBuffers` elements
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct AudioBufferList {
    pub mNumberBuffers: u32,
    pub mBuffers: [AudioBuffer; 1],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct SMPTETime {
    pub mSubframes: i16,
    pub mSubframeDivisor: i16,
    pub mCounter: u32,
    pub mType: u32,
    pub mFlags: u32,
    pub mHours: i16,
    pub mMinutes: i16,
    pub mSeconds: i16,
    pub mFrames: i16,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct AudioTimeStamp {
    pub mSampleTime: f64,
    pub mHostTime: u64,
    pub mRateScalar: f64,
    pub mWordClockTime: u64,
    pub mSMPTETime: SMPTETime,
    pub mFlags: u32,
    pub mReserved: u32,
}

// Compile-time checks of the layouts read through pointers
const _: () = assert!(std::mem::size_of::<AudioTimeStamp>() == 64);
const _: () = assert!(std::mem::size_of::<AudioBuffer>() == 16);
const _: () = assert!(std::mem::size_of::<AudioObjectPropertyAddress>() == 12);
const _: () = assert!(std::mem::size_of::<AudioValueRange>() == 16);

/// The head of `AudioHardwareIOProcStreamUsage`, followed by `mNumberStreams` `u32` flags
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct StreamUsageHead {
    pub mIOProc: *mut c_void,
    pub mNumberStreams: u32,
}

pub type AudioDeviceIOProc = unsafe extern "C" fn(
    AudioObjectID,
    *const AudioTimeStamp,
    *const AudioBufferList,
    *const AudioTimeStamp,
    *mut AudioBufferList,
    *const AudioTimeStamp,
    *mut c_void,
) -> OSStatus;

pub type AudioDeviceIOProcID = *mut c_void;

pub type AudioObjectPropertyListenerProc =
    unsafe extern "C" fn(AudioObjectID, u32, *const AudioObjectPropertyAddress, *mut c_void) -> OSStatus;

#[link(name = "CoreAudio", kind = "framework")]
#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    pub fn AudioObjectGetPropertyDataSize(
        id: AudioObjectID,
        address: *const AudioObjectPropertyAddress,
        qualifier_size: u32,
        qualifier: *const c_void,
        size: *mut u32,
    ) -> OSStatus;
    pub fn AudioObjectGetPropertyData(
        id: AudioObjectID,
        address: *const AudioObjectPropertyAddress,
        qualifier_size: u32,
        qualifier: *const c_void,
        size: *mut u32,
        data: *mut c_void,
    ) -> OSStatus;
    pub fn AudioObjectSetPropertyData(
        id: AudioObjectID,
        address: *const AudioObjectPropertyAddress,
        qualifier_size: u32,
        qualifier: *const c_void,
        size: u32,
        data: *const c_void,
    ) -> OSStatus;
    pub fn AudioObjectAddPropertyListener(
        id: AudioObjectID,
        address: *const AudioObjectPropertyAddress,
        listener: AudioObjectPropertyListenerProc,
        client: *mut c_void,
    ) -> OSStatus;
    pub fn AudioObjectRemovePropertyListener(
        id: AudioObjectID,
        address: *const AudioObjectPropertyAddress,
        listener: AudioObjectPropertyListenerProc,
        client: *mut c_void,
    ) -> OSStatus;
    pub fn AudioDeviceCreateIOProcID(
        device: AudioObjectID,
        proc_: AudioDeviceIOProc,
        client: *mut c_void,
        out: *mut AudioDeviceIOProcID,
    ) -> OSStatus;
    pub fn AudioDeviceDestroyIOProcID(device: AudioObjectID, id: AudioDeviceIOProcID) -> OSStatus;
    pub fn AudioDeviceStart(device: AudioObjectID, id: AudioDeviceIOProcID) -> OSStatus;
    pub fn AudioDeviceStop(device: AudioObjectID, id: AudioDeviceIOProcID) -> OSStatus;
    pub fn AudioHardwareCreateAggregateDevice(description: CFDictionaryRef, out: *mut AudioObjectID) -> OSStatus;
    pub fn AudioHardwareDestroyAggregateDevice(id: AudioObjectID) -> OSStatus;
}

// Keep the CF types referenced for the signatures above
pub type CfString = CFStringRef;
pub type CfType = CFTypeRef;

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct MachTimebaseInfo {
    pub numer: u32,
    pub denom: u32,
}

// libSystem (always linked)
unsafe extern "C" {
    pub fn mach_timebase_info(info: *mut MachTimebaseInfo) -> i32;
    pub fn mach_absolute_time() -> u64;
}
