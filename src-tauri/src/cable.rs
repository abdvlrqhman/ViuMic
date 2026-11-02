//! Gives VB-CABLE's endpoints ViuMic names: apps then list "ViuMic Microphone (VB-Audio Virtual
//! Cable)". Done like Windows' own Sound settings rename devices, through IPolicyConfig, so it
//! needs no admin rights. Runs before every engine start, which also restores the names after a
//! VB-CABLE update or reinstall resets them.

use std::mem::ManuallyDrop;
use viumic_engine::{CABLE_DEVICE, VIUMIC_FEED, VIUMIC_MIC};
use windows::Win32::Devices::FunctionDiscovery::{
    PKEY_Device_DeviceDesc, PKEY_DeviceInterface_FriendlyName,
};
use windows::Win32::Foundation::PROPERTYKEY;
use windows::Win32::Media::Audio::{
    DEVICE_STATE_ACTIVE, IMMDeviceEnumerator, MMDeviceEnumerator, eAll,
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
}

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
