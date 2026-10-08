//! QADREN bootstrap: platform-independent core, with a tiny versioned C ABI.
//! No network, disk or cloud activity is performed by this bootstrap core.

pub mod model;
pub mod registry;
pub mod registry_ffi;
pub mod yandex;
pub mod yandex_ffi;

use std::ffi::c_char;

pub const VERSION: &str = "0.1.0";
pub const ABI_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkerState {
    Idle,
}

pub struct Worker;
impl Worker {
    pub fn state(&self) -> WorkerState {
        WorkerState::Idle
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AdapterCapabilities {
    pub metadata: bool,
    pub read_content: bool,
    pub write_content: bool,
}

pub trait StorageAdapter {
    fn name(&self) -> &'static str;
    fn capabilities(&self) -> AdapterCapabilities;
}

/// Bootstrap adapter for contract testing; does NOT access real storage.
pub struct UnavailableAdapter;
impl StorageAdapter for UnavailableAdapter {
    fn name(&self) -> &'static str {
        "unavailable"
    }
    fn capabilities(&self) -> AdapterCapabilities {
        AdapterCapabilities {
            metadata: false,
            read_content: false,
            write_content: false,
        }
    }
}

static VERSION_C: &[u8] = b"0.1.0\0";
static STATUS_C: &[u8] = b"Core ready - worker idle\0";

/// Returned pointer is static, NUL-terminated, and must never be freed by caller.
#[no_mangle]
pub extern "C" fn qadren_core_version() -> *const c_char {
    VERSION_C.as_ptr().cast()
}

#[no_mangle]
pub extern "C" fn qadren_core_abi_version() -> u32 {
    ABI_VERSION
}

/// Returned pointer is static, NUL-terminated, and must never be freed by caller.
#[no_mangle]
pub extern "C" fn qadren_core_status() -> *const c_char {
    STATUS_C.as_ptr().cast()
}

#[no_mangle]
pub extern "C" fn qadren_worker_is_idle() -> bool {
    Worker.state() == WorkerState::Idle
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn worker_does_not_execute_io() {
        assert_eq!(Worker.state(), WorkerState::Idle);
    }

    #[test]
    fn unavailable_adapter_cannot_transfer() {
        let adapter = UnavailableAdapter;
        assert_eq!(adapter.name(), "unavailable");
        assert!(!adapter.capabilities().read_content);
        assert!(!adapter.capabilities().write_content);
    }

    #[test]
    fn abi_has_stable_version() {
        assert_eq!(qadren_core_abi_version(), 1);
        assert!(!qadren_core_version().is_null());
        assert!(!qadren_core_status().is_null());
    }
}
