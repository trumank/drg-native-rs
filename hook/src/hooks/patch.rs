use core::mem;
use windows::Win32::System::Memory::{
    VirtualProtect, PAGE_EXECUTE_READWRITE, PAGE_PROTECTION_FLAGS,
};

pub struct Patch<T: Copy> {
    address: *mut T,
    original: T,
}

impl<T: Copy> Patch<T> {
    pub unsafe fn new(address: *mut T, new_value: T) -> Patch<T> {
        let original = *address;

        Self::write(address, new_value);

        Patch { address, original }
    }

    unsafe fn write(address: *mut T, new_value: T) {
        let mut old_protection: PAGE_PROTECTION_FLAGS = Default::default();
        VirtualProtect(
            address.cast(),
            mem::size_of::<T>(),
            PAGE_EXECUTE_READWRITE,
            &mut old_protection,
        );
        *address = new_value;
        VirtualProtect(
            address.cast(),
            mem::size_of::<T>(),
            old_protection,
            &mut old_protection,
        );
    }
}

impl<T: Copy> Drop for Patch<T> {
    fn drop(&mut self) {
        unsafe {
            Self::write(self.address, self.original);
        }
    }
}
