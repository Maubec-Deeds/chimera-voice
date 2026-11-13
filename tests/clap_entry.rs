//! The built library is a CLAP plugin a host can find: it exports `clap_entry`, and its factory
//! lists Chimera Voice. Run after `cargo build --release` (the test loads that DLL; it is skipped
//! when the DLL has not been built).
#![cfg(windows)]

use std::ffi::{c_char, c_void, CStr, CString};

#[repr(C)]
struct ClapVersion {
    major: u32,
    minor: u32,
    revision: u32,
}

#[repr(C)]
struct ClapEntry {
    version: ClapVersion,
    init: unsafe extern "C" fn(*const c_char) -> bool,
    deinit: unsafe extern "C" fn(),
    get_factory: unsafe extern "C" fn(*const c_char) -> *const c_void,
}

#[repr(C)]
struct ClapFactory {
    get_plugin_count: unsafe extern "C" fn(*const ClapFactory) -> u32,
    get_plugin_descriptor: unsafe extern "C" fn(*const ClapFactory, u32) -> *const ClapDescriptor,
}

#[repr(C)]
struct ClapDescriptor {
    version: ClapVersion,
    id: *const c_char,
    name: *const c_char,
    vendor: *const c_char,
    url: *const c_char,
    manual_url: *const c_char,
    support_url: *const c_char,
    plugin_version: *const c_char,
    description: *const c_char,
    features: *const *const c_char,
}

extern "system" {
    fn LoadLibraryW(name: *const u16) -> *mut c_void;
    fn GetProcAddress(module: *mut c_void, name: *const c_char) -> *mut c_void;
}

#[test]
fn a_host_finds_the_plugin() {
    let dll = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target").join("release").join("chimera_voice.dll");
    if !dll.exists() {
        eprintln!("skipped: {} is not built", dll.display());
        return;
    }
    let wide: Vec<u16> = dll.to_string_lossy().encode_utf16().chain(std::iter::once(0)).collect();
    unsafe {
        let module = LoadLibraryW(wide.as_ptr());
        assert!(!module.is_null(), "the DLL does not load");
        let entry = GetProcAddress(module, c"clap_entry".as_ptr()) as *const ClapEntry;
        assert!(!entry.is_null(), "no clap_entry export");
        let entry = &*entry;
        assert_eq!(entry.version.major, 1);
        let path = CString::new(dll.to_string_lossy().as_bytes()).unwrap();
        assert!((entry.init)(path.as_ptr()));
        let factory = (entry.get_factory)(c"clap.plugin-factory".as_ptr()) as *const ClapFactory;
        assert!(!factory.is_null());
        assert_eq!(((*factory).get_plugin_count)(factory), 1);
        let d = &*((*factory).get_plugin_descriptor)(factory, 0);
        assert_eq!(CStr::from_ptr(d.id).to_str().unwrap(), "com.chimera-voice.chimera-voice");
        assert_eq!(CStr::from_ptr(d.name).to_str().unwrap(), "Chimera Voice");
        let mut features = Vec::new();
        let mut f = d.features;
        while !(*f).is_null() {
            features.push(CStr::from_ptr(*f).to_str().unwrap().to_string());
            f = f.add(1);
        }
        assert!(features.iter().any(|x| x == "instrument"), "{features:?}");
        (entry.deinit)();
    }
}
