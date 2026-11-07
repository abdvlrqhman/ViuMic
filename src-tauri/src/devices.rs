//! Windows audio device helpers, all done the way Windows' own Sound settings do them
//! (IPolicyConfig, IAudioEndpointVolume), so none need admin rights:
//! - `brand`: gives VB-CABLE's endpoints ViuMic names ("ViuMic Microphone (VB-Audio Virtual
//!   Cable)" in apps). Runs before every engine start, which also restores the names after a
//!   VB-CABLE update or reinstall resets them.
//! - `any_muted`: whether the user muted their mic or ViuMic's (laptop mute key, taskbar).
//! - `defaults` / `make_default_mic` / `fix_speaker`: the default-device problems VB-CABLE's
//!   install causes (it makes itself the default mic and the communications speaker).

use std::mem::ManuallyDrop;
use viumic_engine::{CABLE_DEVICE, VIUMIC_FEED, VIUMIC_MIC};
use windows::Win32::Devices::FunctionDiscovery::{
    PKEY_Device_DeviceDesc, PKEY_Device_FriendlyName, PKEY_DeviceInterface_FriendlyName,
};
use windows::Win32::Foundation::PROPERTYKEY;
use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
use windows::Win32::Media::Audio::{
    DEVICE_STATE_ACTIVE, EDataFlow, ERole, IMMDevice, IMMDeviceEnumerator, MMDeviceEnumerator,
    eAll, eCapture, eCommunications, eConsole, eMultimedia, eRender,
};
use windows::Win32::System::Com::StructuredStorage::PROPVARIANT;
use windows::Win32::System::Com::{
    CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoTaskMemFree, STGM_READ,
};
use windows::Win32::System::Variant::VT_LPWSTR;
use windows::core::{BOOL, GUID, HRESULT, IUnknown, IUnknown_Vtbl, PCWSTR, PWSTR, interface};

/// VB-CABLE's stock endpoint names, and what ViuMic calls them.
const RENAMES: [(&str, &str); 3] = [
    ("CABLE Output", VIUMIC_MIC),
    ("CABLE Input", VIUMIC_FEED),
    ("CABLE In 16 Ch", "ViuMic Spare 16ch"),
];

/// Undocumented but stable since Windows 7 (used by Windows' Sound settings and tools like
/// SoundSwitch). Only the vtable order matters for the methods ViuMic doesn't call.
#[interface("f8679f50-850a-41cf-9c72-430f290290c8")]
unsafe trait IPolicyConfig: IUnknown {
    fn get_mix_format(&self) -> HRESULT;
    fn get_device_format(&self) -> HRESULT;
    fn reset_device_format(&self) -> HRESULT;
    fn set_device_format(&self) -> HRESULT;
    fn get_processing_period(&self) -> HRESULT;
    fn set_processing_period(&self) -> HRESULT;
    fn get_share_mode(&self) -> HRESULT;
    fn set_share_mode(&self) -> HRESULT;
    fn get_property_value(
        &self,
        id: PCWSTR,
        fx: BOOL,
        key: *const PROPERTYKEY,
        value: *mut PROPVARIANT,
    ) -> HRESULT;
    fn set_property_value(
        &self,
        id: PCWSTR,
        fx: BOOL,
        key: *const PROPERTYKEY,
        value: *const PROPVARIANT,
    ) -> HRESULT;
    fn set_default_endpoint(&self, id: PCWSTR, role: ERole) -> HRESULT;
}

/// Every default-device role Windows tracks: apps, media, and calls.
const ROLES: [ERole; 3] = [eConsole, eMultimedia, eCommunications];

const POLICY_CONFIG_CLIENT: GUID = GUID::from_u128(0x870af99c_171d_4f9e_af0d_e63df40c2bc9);

/// Renames every VB-CABLE endpoint that still has its stock name. Returns how many changed.
pub fn brand() -> windows::core::Result<usize> {
    unsafe {
        // Already initialised on this thread (S_FALSE / RPC_E_CHANGED_MODE) is fine too.
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let devices: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
        let policy: IPolicyConfig = CoCreateInstance(&POLICY_CONFIG_CLIENT, None, CLSCTX_ALL)?;
        let endpoints = devices.EnumAudioEndpoints(eAll, DEVICE_STATE_ACTIVE)?;
        let mut renamed = 0;
        for i in 0..endpoints.GetCount()? {
            let endpoint = endpoints.Item(i)?;
            let props = endpoint.OpenPropertyStore(STGM_READ)?;
            if props
                .GetValue(&PKEY_DeviceInterface_FriendlyName)?
                .to_string()
                != CABLE_DEVICE
            {
                continue;
            }
            let name = props.GetValue(&PKEY_Device_DeviceDesc)?.to_string();
            let Some((_, new)) = RENAMES.iter().find(|(old, _)| *old == name) else {
                continue;
            };

            // A VT_LPWSTR value pointing at our own buffer: wrapped in ManuallyDrop so
            // PROPVARIANT's Drop never frees memory it doesn't own.
            let wide: Vec<u16> = new.encode_utf16().chain([0]).collect();
            let mut value = ManuallyDrop::new(PROPVARIANT::default());
            (*value.Anonymous.Anonymous).vt = VT_LPWSTR;
            (*value.Anonymous.Anonymous).Anonymous.pwszVal = PWSTR(wide.as_ptr().cast_mut());

            let id = endpoint.GetId()?;
            let result =
                policy.set_property_value(PCWSTR(id.0), BOOL(0), &PKEY_Device_DeviceDesc, &*value);
            CoTaskMemFree(Some(id.0.cast()));
            result.ok()?;
            renamed += 1;
        }
        Ok(renamed)
    }
}

fn com() -> windows::core::Result<IMMDeviceEnumerator> {
    unsafe {
        // Already initialised on this thread (S_FALSE / RPC_E_CHANGED_MODE) is fine too.
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)
    }
}

fn name(device: &IMMDevice) -> windows::core::Result<String> {
    unsafe {
        let props = device.OpenPropertyStore(STGM_READ)?;
        Ok(props.GetValue(&PKEY_Device_FriendlyName)?.to_string())
    }
}

/// Active endpoints of one direction, with their friendly names.
fn endpoints(flow: EDataFlow) -> windows::core::Result<Vec<(String, IMMDevice)>> {
    unsafe {
        let all = com()?.EnumAudioEndpoints(flow, DEVICE_STATE_ACTIVE)?;
        (0..all.GetCount()?)
            .map(|i| {
                let d = all.Item(i)?;
                Ok((name(&d)?, d))
            })
            .collect()
    }
}

fn default_name(flow: EDataFlow, role: ERole) -> Option<String> {
    unsafe {
        com()
            .ok()?
            .GetDefaultAudioEndpoint(flow, role)
            .ok()
            .and_then(|d| name(&d).ok())
    }
}

/// Whether any capture endpoint matching `which` (by friendly name) is muted in Windows.
pub fn any_muted(which: impl Fn(&str) -> bool) -> bool {
    let Ok(mics) = endpoints(eCapture) else {
        return false;
    };
    mics.iter().filter(|(n, _)| which(n)).any(|(_, d)| unsafe {
        d.Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None)
            .and_then(|v| v.GetMute())
            .is_ok_and(|m| m.as_bool())
    })
}

pub struct Defaults {
    /// "ViuMic Microphone" is the default mic for apps and for calls.
    pub mic_is_viumic: bool,
    /// "ViuMic Feed" is a default speaker for some role: that audio vanishes into the cable.
    pub feed_is_speaker: bool,
}

pub fn defaults() -> Defaults {
    let is = |flow, role, what: &str| default_name(flow, role).is_some_and(|n| n.contains(what));
    Defaults {
        mic_is_viumic: [eConsole, eCommunications]
            .into_iter()
            .all(|r| is(eCapture, r, VIUMIC_MIC)),
        feed_is_speaker: ROLES.into_iter().any(|r| is(eRender, r, CABLE_DEVICE)),
    }
}

fn set_default(device: &IMMDevice, roles: &[ERole]) -> windows::core::Result<()> {
    unsafe {
        let policy: IPolicyConfig = CoCreateInstance(&POLICY_CONFIG_CLIENT, None, CLSCTX_ALL)?;
        let id = device.GetId()?;
        let result = roles
            .iter()
            .try_for_each(|&role| policy.set_default_endpoint(PCWSTR(id.0), role).ok());
        CoTaskMemFree(Some(id.0.cast()));
        result
    }
}

/// Makes "ViuMic Microphone" the default mic for every role, so laptop mute keys and apps set to
/// "Default" use ViuMic.
pub fn make_default_mic() -> windows::core::Result<()> {
    let mics = endpoints(eCapture)?;
    let Some((_, viumic)) = mics.iter().find(|(n, _)| n.contains(VIUMIC_MIC)) else {
        return Err(windows::core::Error::from_hresult(
            windows::Win32::Foundation::E_FAIL,
        ));
    };
    set_default(viumic, &ROLES)
}

/// Moves every speaker role off the cable: to the main default speaker if that's a real one,
/// else to the first real output.
pub fn fix_speaker() -> windows::core::Result<()> {
    let outputs = endpoints(eRender)?;
    let real = |n: &String| !n.contains(CABLE_DEVICE);
    let main = default_name(eRender, eConsole).filter(real);
    let Some((_, target)) = outputs
        .iter()
        .find(|(n, _)| Some(n) == main.as_ref())
        .or_else(|| outputs.iter().find(|(n, _)| real(n)))
    else {
        return Ok(()); // no real speakers to move to
    };
    let stuck: Vec<ERole> = ROLES
        .into_iter()
        .filter(|&r| default_name(eRender, r).is_some_and(|n| n.contains(CABLE_DEVICE)))
        .collect();
    set_default(target, &stuck)
}
