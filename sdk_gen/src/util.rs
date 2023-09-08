#[macro_export]
macro_rules! sdk_file {
    ($filename:literal) => {{
        concat!(sdk_path!(), '/', $filename)
    }};
}

#[macro_export]
macro_rules! sdk_path {
    () => {
        include_str!(concat!(env!("OUT_DIR"), "/sdk_path"))
    };
}
