use windows::Win32::Foundation::HMODULE;
use windows::Win32::System::LibraryLoader::DisableThreadLibraryCalls;

pub mod module;
pub use module::Module;

pub mod random;

pub const DLL_PROCESS_DETACH: u32 = 0;
pub const DLL_PROCESS_ATTACH: u32 = 1;
pub const STD_OUTPUT_HANDLE: u32 = 0xFFFF_FFF5;
pub const STD_INPUT_HANDLE: u32 = 0xFFFF_FFF6;

type ThreadProc = unsafe extern "system" fn(parameter: HMODULE) -> u32;

pub unsafe fn dll_main(
    dll: HMODULE,
    reason: u32,
    on_attach: ThreadProc,
    on_detach: unsafe fn(),
) -> i32 {
    if reason == DLL_PROCESS_ATTACH {
        DisableThreadLibraryCalls(dll);
        std::thread::spawn(move || unsafe {
            std::thread::sleep(std::time::Duration::from_secs(10));
            on_attach(dll)
        });
    } else if reason == DLL_PROCESS_DETACH {
        on_detach();
    }

    1
}

pub unsafe fn idle() {}
