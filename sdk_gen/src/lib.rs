use common::{list, win, GUObjectArray, Hex, NamePoolData, Timer};
use std::io::{BufWriter, Write};
use windows::Win32::{Foundation::HMODULE, System::LibraryLoader::FreeLibraryAndExitThread};

mod game;
mod generator;
use generator::Generator;
mod util;

#[derive(macros::NoPanicErrorDebug)]
enum Error {
    Game(#[from] game::Error),
    Module(#[from] win::module::Error),
    List(#[from] list::Error),
    Generator(#[from] generator::Error),
    Common(#[from] common::Error),
    Io(#[from] std::io::Error),
}

#[no_mangle]
#[allow(non_snake_case, unused_variables)]
unsafe extern "system" fn DllMain(dll: HMODULE, reason: u32, _: *mut ()) -> i32 {
    win::dll_main(dll, reason, on_attach, on_detach)
}

unsafe extern "system" fn on_attach(dll: HMODULE) -> u32 {
    if let Err(e) = run() {
        common::log!("error: {:?}", e);
        common::idle();
    }

    FreeLibraryAndExitThread(dll, 0);
}

unsafe fn on_detach() {}

unsafe fn run() -> Result<(), Error> {
    common::init_globals(&win::Module::current()?)?;
    dump_globals()?;

    if cfg!(feature = "gen_sdk") {
        generate_sdk()?;
    }

    common::idle();
    Ok(())
}

unsafe fn dump_globals() -> Result<(), Error> {
    let timer = Timer::new("dump global names and objects");
    dump_names()?;
    dump_objects()?;
    timer.stop();
    Ok(())
}

unsafe fn dump_names() -> Result<(), Error> {
    let mut file = BufWriter::new(std::fs::File::create(sdk_file!("global_names.txt"))?);

    for (index, name) in (*NamePoolData).iter() {
        let text = (*name).text();
        writeln!(&mut file, "[{}] {}", index.value(), text)?;
    }

    Ok(())
}

unsafe fn dump_objects() -> Result<(), Error> {
    let mut file = BufWriter::new(std::fs::File::create(sdk_file!("global_objects.txt"))?);

    for object in (*GUObjectArray).iter().filter(|o| !o.is_null()) {
        writeln!(
            &mut file,
            "[{}] {} {}",
            (*object).InternalIndex,
            *object,
            Hex(object)
        )?;
    }

    Ok(())
}

unsafe fn generate_sdk() -> Result<(), Error> {
    let timer = Timer::new("generate sdk");
    Generator::new()?.generate_sdk()?;
    timer.stop();
    Ok(())
}
