#![no_std]

// // https://docs.microsoft.com/en-us/cpp/c-runtime-library/crt-library-features?view=msvc-160
// #[link(name = "ucrt")]
// extern {}

#[link(name = "msvcrt")]
extern "C" {}

#[link(name = "vcruntime")]
extern "C" {}

use common::{self, win, EClassCastFlags, List, UFunction, UObject};
use core::ffi::c_void;
use core::mem::{self, ManuallyDrop};
use core::ptr;
use core::slice;
use sdk::Engine::{Actor, Engine};

#[derive(macros::NoPanicErrorDebug)]
enum Error {
    Common(#[from] common::Error),
    Module(#[from] win::module::Error),
    NoCodeCave,
    FindProcessEvent,
    FindGlobalEngine,
}

#[allow(non_upper_case_globals)]
static mut GEngine: *const Engine = ptr::null();

#[no_mangle]
unsafe extern "system" fn _DllMainCRTStartup(dll: *mut c_void, reason: u32, _: *mut c_void) -> i32 {
    win::dll_main(dll, reason, on_attach, on_detach)
}

unsafe extern "system" fn on_attach(dll: *mut c_void) -> u32 {
    win::AllocConsole();

    if let Err(e) = run() {
        common::log!("error: {:?}", e);
        common::idle();
    }

    win::FreeConsole();
    win::FreeLibraryAndExitThread(dll, 0);
    0
}

struct Patch<const N: usize> {
    address: *mut u8,
    original_bytes: [u8; N],
}

impl<const N: usize> Patch<N> {
    pub unsafe fn new(address: *mut u8, new_bytes: [u8; N]) -> Patch<N> {
        let mut original_bytes = [0; N];
        (&mut original_bytes).copy_from_slice(slice::from_raw_parts(address, N));

        Self::write(address, new_bytes);

        Patch {
            address,
            original_bytes,
        }
    }

    unsafe fn write(address: *mut u8, bytes: [u8; N]) {
        const PAGE_EXECUTE_READWRITE: u32 = 0x40;
        let mut old_protection = 0;
        win::VirtualProtect(
            address.cast(),
            N,
            PAGE_EXECUTE_READWRITE,
            &mut old_protection,
        );
        slice::from_raw_parts_mut(address, N).copy_from_slice(&bytes);
        win::VirtualProtect(address.cast(), N, old_protection, &mut old_protection);
        win::FlushInstructionCache(win::GetCurrentProcess(), address.cast(), N);
    }
}

impl<const N: usize> Drop for Patch<N> {
    fn drop(&mut self) {
        unsafe {
            Self::write(self.address, self.original_bytes);
        }
    }
}

struct ProcessEventHook {
    jmp: ManuallyDrop<Patch<6>>,
    code_cave: ManuallyDrop<Patch<31>>,
}

impl Drop for ProcessEventHook {
    fn drop(&mut self) {
        unsafe {
            ManuallyDrop::drop(&mut self.jmp);
            // Before we destroy the code cave, give the CPU time to exit the cave.
            win::Sleep(100);
            ManuallyDrop::drop(&mut self.code_cave);
        }
    }
}

impl ProcessEventHook {
    pub unsafe fn new(process_event: *mut u8, code_cave: &mut [u8]) -> ProcessEventHook {
        let code_cave_patch = {
            let mut patch = [
                // push rcx
                0x51,

                // push rdx
                0x52, 
                
                // push r8
                0x41, 0x50,
                
                // mov rax, my_process_event (need to fill in)
                0x48, 0xB8, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
                
                // call rax
                0xFF, 0xD0,
                
                // pop r8
                0x41, 0x58,
                
                // pop rdx
                0x5A,
                
                // pop rcx
                0x59,
                
                // first six bytes of ProcessEvent (need to fill in)
                0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
                
                // jmp ProcessEvent+6 (need to fill in)
                0xE9, 0x00, 0x00, 0x00, 0x00,
            ];

            // mov rax, my_process_event
            (&mut patch[6..6 + mem::size_of::<usize>()])
                .copy_from_slice(&(my_process_event as usize).to_le_bytes());

            // first six bytes of ProcessEvent
            let first_six_process_event_bytes = slice::from_raw_parts(process_event, 6);
            (&mut patch[20..20 + first_six_process_event_bytes.len()])
                .copy_from_slice(first_six_process_event_bytes);

            // jmp ProcessEvent+6
            let patch_len = patch.len();
            (&mut patch[27..27 + mem::size_of::<u32>()]).copy_from_slice({
                let destination = process_event as usize + first_six_process_event_bytes.len();
                let source = code_cave.as_ptr() as usize + patch_len;
                let relative_distance = destination.wrapping_sub(source) as u32;
                &relative_distance.to_le_bytes()
            });

            patch
        };

        let jmp_patch = {
            let mut patch = [
                // jmp code_cave (need to fill in)
                0xE9, 0x00, 0x00, 0x00, 0x00,
                // nop (otherwise we would cut a two byte instruction in half)
                0x90,
            ];

            let destination = code_cave.as_ptr() as usize;
            let source = process_event as usize + 5;
            let relative_distance = destination.wrapping_sub(source) as u32;
            (&mut patch[1..1 + mem::size_of::<u32>()])
                .copy_from_slice(&relative_distance.to_le_bytes());

            patch
        };

        ProcessEventHook {
            jmp: ManuallyDrop::new(Patch::new(process_event, jmp_patch)),
            code_cave: ManuallyDrop::new(Patch::new(code_cave.as_mut_ptr(), code_cave_patch)),
        }
    }
}

unsafe fn run() -> Result<(), Error> {
    let module = win::Module::current()?;

    init_globals(&module)?;

    let code_cave = module.find_code_cave().ok_or(Error::NoCodeCave)?;
    let cave_size = code_cave.len();

    common::log!(
        "Module starts at {} and is {} bytes.\n\
        Largest code cave begins at {} and is {} bytes.\n\
        my_process_event is at {}",
        module.start(),
        module.size(),
        code_cave.as_ptr() as usize,
        cave_size,
        my_process_event as usize,
    );

    let process_event = module
        .find_mut(&[
            Some(0x40),
            Some(0x55),
            Some(0x56),
            Some(0x57),
            Some(0x41),
            Some(0x54),
            Some(0x41),
            Some(0x55),
            Some(0x41),
            Some(0x56),
            Some(0x41),
            Some(0x57),
            Some(0x48),
            Some(0x81),
            Some(0xEC),
            Some(0xF0),
            Some(0x00),
            Some(0x00),
            Some(0x00),
        ])
        .ok_or(Error::FindProcessEvent)?;

    let _process_event_hook = ProcessEventHook::new(process_event, code_cave);

    common::idle();

    for &function in RESET_THESE_SEEN_COUNTS.iter() {
        (*function).seen_count = 0;
    }

    Ok(())
}

unsafe fn on_detach() {}

unsafe fn init_globals(module: &win::Module) -> Result<(), Error> {
    common::init_globals(module)?;
    find_global_engine(module)?;
    Ok(())
}

unsafe fn find_global_engine(module: &win::Module) -> Result<(), Error> {
    // 00007FF63919DE6E   48:8B0D 137D3204   mov rcx,qword ptr ds:[7FF63D4C5B88]
    // 00007FF63919DE75   49:8BD7            mov rdx,r15
    // 00007FF63919DE78   48:8B01            mov rax,qword ptr ds:[rcx]
    // 00007FF63919DE7B   FF90 80020000      call qword ptr ds:[rax+280]
    const PATTERN: [Option<u8>; 19] = [
        Some(0x48),
        Some(0x8B),
        Some(0x0D),
        None,
        None,
        None,
        None,
        Some(0x49),
        Some(0x8B),
        Some(0xD7),
        Some(0x48),
        Some(0x8B),
        Some(0x01),
        Some(0xFF),
        Some(0x90),
        Some(0x80),
        Some(0x02),
        Some(0x00),
        Some(0x00),
    ];
    let mov_rcx_global_engine: *const u8 = module.find(&PATTERN).ok_or(Error::FindGlobalEngine)?;
    let relative_offset = mov_rcx_global_engine.add(3).cast::<u32>().read_unaligned();
    GEngine = *mov_rcx_global_engine
        .add(7 + relative_offset as usize)
        .cast::<*const Engine>();
    common::log!("GEngine = {}", GEngine as usize);
    Ok(())
}

static mut RESET_THESE_SEEN_COUNTS: List<*mut UFunction, 4096> = List::new();

unsafe extern "C" fn my_process_event(
    object: *mut UObject,
    function: *mut UFunction,
    _parameters: *mut c_void,
) {
    const MAX_PRINTS: u32 = 1;

    let seen_count = (*function).seen_count;

    if seen_count == 0 && RESET_THESE_SEEN_COUNTS.push(function).is_err() {
        common::log!("Warning: RESET_THESE_SEEN_COUNTS reached its max capacity of {}. We won't print any more unseen UFunctions.", RESET_THESE_SEEN_COUNTS.capacity());
        return;
    }

    if seen_count < MAX_PRINTS {
        (*function).seen_count += 1;

        let is_actor = (*object).fast_is(EClassCastFlags::CASTCLASS_AActor);

        common::log!(
            "{}{}\n\t{}",
            if is_actor { "\n" } else { "" },
            (*object).name(),
            *function
        );

        if is_actor {
            let mut owner = (*object.cast::<Actor>()).Owner;

            while !owner.is_null() {
                common::log!("owned by\n\t{}", (*owner.cast::<UObject>()).name());
                owner = (*owner).Owner;
            }

            common::log!();
        }
    }
}
