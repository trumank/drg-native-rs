use crate::split::ReverseSplitIterator;
use crate::win;
use crate::FName;
use crate::List;

use core::convert::TryFrom;
use core::ffi::c_void;
use core::fmt::{self, Display, Formatter};
use core::mem;
use core::ops::BitOr;
use core::ptr;
use core::str;

mod full_name;
use full_name::FullName;

pub static mut GUObjectArray: *const FUObjectArray = ptr::null();

const NumElementsPerChunk: usize = 64 * 1024;

// The maximum number of outers we can store in an array.
// Set to a large enough number to cover the outers length of all objects.
// Used when constructing an object's name, as well as for name comparisons.
const MAX_OUTERS: usize = 32;

#[derive(macros::NoPanicErrorDebug)]
pub enum Error {
    FindGUObjectArray,
    Fmt(#[from] fmt::Error),
    FullName(#[from] full_name::Error),
    UnableToFind(&'static str),
}

#[repr(C)]
pub struct FUObjectArray {
    ObjFirstGCIndex: i32,
    ObjLastNonGCIndex: i32,
    MaxObjectsNotConsideredByGC: i32,
    OpenForDisregardForGC: bool,
    pub ObjObjects: TUObjectArray,
}

impl FUObjectArray {
    pub unsafe fn init(module: &win::Module) -> Result<(), Error> {
        // https://github.com/rkr35/drg/issues/3

        // 00007FF75CAF6D32 | 48:8B05 F7845C04         | mov rax,qword ptr ds:[7FF7610BF230]     |
        // 00007FF75CAF6D39 | 48:8B0CC8                | mov rcx,qword ptr ds:[rax+rcx*8]        |
        // 00007FF75CAF6D3D | 4C:8D04D1                | lea r8,qword ptr ds:[rcx+rdx*8]         |

        const GU_OBJECT_ARRAY_PATTERN: [Option<u8>; 15] = [
            Some(0x48),
            Some(0x8B),
            Some(0x05),
            None,
            None,
            None,
            None,
            Some(0x48),
            Some(0x8B),
            Some(0x0C),
            Some(0xC8),
            Some(0x4C),
            Some(0x8D),
            Some(0x04),
            Some(0xD1),
        ];

        let mov_rax: *const u8 = module
            .find(&GU_OBJECT_ARRAY_PATTERN)
            .ok_or(Error::FindGUObjectArray)?;

        let mov_immediate = mov_rax.add(3);
        let instruction_after_mov = mov_immediate.add(4);
        let mov_immediate = mov_immediate.cast::<u32>().read_unaligned();

        GUObjectArray = instruction_after_mov
            .add(mov_immediate as usize)
            .sub(0x10)
            .cast();

        Ok(())
    }

    #[inline(never)]
    pub unsafe fn find_function(&self, name: &'static str) -> *mut UFunction {
        self.find(name)
            .map(|f| f.cast())
            .unwrap_or(core::ptr::null_mut())
    }

    pub unsafe fn find(&self, name: &'static str) -> Result<*mut UObject, Error> {
        // Do a short-circuiting name comparison.

        // Compare the class from `name` against the class in `self`.
        // Then compare the outers in `name` against the outers in `self`.

        // This way, we don't have to construct the full name of `self` if we
        // can rule out non-matching classes and outers sooner.

        let target = FullName::<MAX_OUTERS>::try_from(name)?;

        'outer: for object in self.iter() {
            if object.is_null() {
                // We're not looking for a null object.
                continue;
            }

            let my_name = (*object).name().as_bytes();

            if my_name != target.name {
                // Object names don't match.
                // No need to check the class. Let's bail.
                continue;
            }

            let my_class = (*(*object).ClassPrivate).name().as_bytes();

            if my_class != target.class {
                // Classes don't match.
                // No need to check the outers. Let's bail.
                continue;
            }

            let mut my_outer = (*object).OuterPrivate;

            for target_outer in target.outers.iter() {
                if my_outer.is_null() {
                    // We have no more outers left to check for this object, but
                    // we still have target outers. So this object can't be what
                    // we're looking for. Let's check out the next object.
                    continue 'outer;
                }

                let my_outer_name = (*my_outer).name().as_bytes();

                if my_outer_name != *target_outer {
                    // This outer doesn't match the target outer we're looking for.
                    // No need to check the remaining outers. Let's bail.
                    continue 'outer;
                }

                // Advance up to the next outer.
                my_outer = (*my_outer).OuterPrivate;
            }

            // We got here because the name, class, and outers all match the
            // input name. So our search is over.
            return Ok(object);
        }

        // No object matched our search.
        Err(Error::UnableToFind(name))
    }

    pub unsafe fn index_to_object(&self, index: i32) -> *const FUObjectItem {
        if index < self.ObjObjects.NumElements {
            let index = index as usize;
            let chunk = *self.ObjObjects.Objects.add(index / NumElementsPerChunk);
            chunk.add(index % NumElementsPerChunk)
        } else {
            ptr::null()
        }
    }

    pub fn iter(&self) -> ObjectIterator {
        ObjectIterator {
            chunks: self.ObjObjects.Objects,
            num_objects: self.ObjObjects.NumElements as usize,
            index: 0,
        }
    }
}

pub struct ObjectIterator {
    chunks: *const *mut FUObjectItem,
    num_objects: usize,
    index: usize,
}

impl Iterator for ObjectIterator {
    type Item = *mut UObject;

    fn next(&mut self) -> Option<Self::Item> {
        unsafe {
            if self.index < self.num_objects {
                let chunk = *self.chunks.add(self.index / NumElementsPerChunk);
                let object = chunk.add(self.index % NumElementsPerChunk);
                let object = (*object).Object;
                self.index += 1;
                Some(object)
            } else {
                None
            }
        }
    }
}

#[repr(C)]
pub struct TUObjectArray {
    Objects: *const *mut FUObjectItem,
    PreAllocatedObjects: *mut FUObjectItem,
    MaxElements: i32,
    NumElements: i32,
    MaxChunks: i32,
    NumChunks: i32,
}

#[repr(C)]
pub struct FUObjectItem {
    pub Object: *mut UObject,
    Flags: i32,
    ClusterRootIndex: i32,
    pub SerialNumber: i32,
}

impl FUObjectItem {
    pub fn is_unreachable(&self) -> bool {
        const UNREACHABLE: i32 = 1 << 28;
        self.Flags & UNREACHABLE == UNREACHABLE
    }

    pub fn is_pending_kill(&self) -> bool {
        const PENDING_KILL: i32 = 1 << 29;
        self.Flags & PENDING_KILL == PENDING_KILL
    }

    pub fn is_valid(&self) -> bool {
        !self.is_unreachable() && !self.is_pending_kill()
    }
}

#[macro_export]
macro_rules! impl_deref {
    ($Derived:ty as $Base:ty) => {
        impl core::ops::Deref for $Derived {
            type Target = $Base;

            fn deref(&self) -> &Self::Target {
                &self.base
            }
        }

        impl core::ops::DerefMut for $Derived {
            fn deref_mut(&mut self) -> &mut Self::Target {
                &mut self.base
            }
        }

        impl core::fmt::Display for $Derived {
            fn fmt(&self, f: &mut core::fmt::Formatter) -> Result<(), core::fmt::Error> {
                let object: &UObject = self;
                object.fmt(f)
            }
        }
    };
}

#[repr(C)]
pub struct UObject {
    pub vtable: *mut *const c_void,
    ObjectFlags: u32, //EObjectFlags
    pub InternalIndex: i32,
    ClassPrivate: *const UClass,
    pub NamePrivate: FName,
    OuterPrivate: *mut UObject,
}

impl UObject {
    pub unsafe fn package(&self) -> *const UPackage {
        let mut top = self as *const UObject;

        while !(*top).OuterPrivate.is_null() {
            top = (*top).OuterPrivate;
        }

        top.cast()
    }

    pub unsafe fn package_mut(&mut self) -> *mut UPackage {
        let mut top = self as *mut UObject;

        while !(*top).OuterPrivate.is_null() {
            top = (*top).OuterPrivate;
        }

        top.cast()
    }

    pub unsafe fn is(&self, class: *const UClass) -> bool {
        (*self.ClassPrivate).is(class.cast())
    }

    pub unsafe fn fast_is(&self, class: EClassCastFlags) -> bool {
        (*self.ClassPrivate).ClassCastFlags.any(class)
    }

    pub unsafe fn name(&self) -> &str {
        self.NamePrivate.text()
    }

    pub unsafe fn process_event(
        this: *mut UObject,
        function: *mut UFunction,
        parameters: *mut c_void,
    ) {
        // 00007FF6389DDFA0 | 48:895C24 08             | mov qword ptr ss:[rsp+8],rbx            |
        // 00007FF6389DDFA5 | 57                       | push rdi                                |
        // 00007FF6389DDFA6 | 48:83EC 20               | sub rsp,20                              |
        // 00007FF6389DDFAA | 48:8B15 97474B02         | mov rdx,qword ptr ds:[7FF63AE92748]     |
        // 00007FF6389DDFB1 | 48:8BF9                  | mov rdi,rcx                             |
        // 00007FF6389DDFB4 | 48:8B19                  | mov rbx,qword ptr ds:[rcx]              |
        // 00007FF6389DDFB7 | F3:0F114C24 38           | movss dword ptr ss:[rsp+38],xmm1        |
        // 00007FF6389DDFBD | E8 7E5C38FE              | call fsd-win64-shipping.7FF636D63C40    |
        // 00007FF6389DDFC2 | 48:8BD0                  | mov rdx,rax                             |
        // 00007FF6389DDFC5 | 4C:8D4424 38             | lea r8,qword ptr ss:[rsp+38]            |
        // 00007FF6389DDFCA | 48:8BCF                  | mov rcx,rdi                             |
        // 00007FF6389DDFCD | FF93 20020000            | call qword ptr ds:[rbx+220]             | <<<< 0x220 / 8 = 0x44 = 68
        // 00007FF6389DDFD3 | 48:8B5C24 30             | mov rbx,qword ptr ss:[rsp+30]           |
        // 00007FF6389DDFD8 | 48:83C4 20               | add rsp,20                              |
        // 00007FF6389DDFDC | 5F                       | pop rdi                                 |
        // 00007FF6389DDFDD | C3                       | ret                                     |
        const PROCESS_EVENT_VTABLE_INDEX: usize = 68;

        type ProcessEvent = unsafe extern "C" fn(*mut UObject, *mut UFunction, *mut c_void);
        let process_event = mem::transmute::<*const c_void, ProcessEvent>(
            *(*this).vtable.add(PROCESS_EVENT_VTABLE_INDEX),
        );
        process_event(this, function, parameters);
    }
}

impl Display for UObject {
    fn fmt(&self, f: &mut Formatter) -> Result<(), fmt::Error> {
        unsafe {
            write!(f, "{} ", (*self.ClassPrivate).name())?;

            let mut outers = List::<&str, MAX_OUTERS>::new();
            let mut outer = self.OuterPrivate;

            while !outer.is_null() {
                if outers.push((*outer).name()).is_err() {
                    crate::log!("warning: reached outers capacity of {} for {}. outer name will be truncated.", outers.capacity(), self as *const _ as usize);
                    break;
                }

                outer = (*outer).OuterPrivate;
            }

            for outer in outers.iter().rev() {
                write!(f, "{}.", outer)?;
            }

            write!(f, "{}", self.name())?;

            if self.NamePrivate.number() > 0 {
                write!(f, "_{}", self.NamePrivate.number() - 1)?;
            }
        }

        Ok(())
    }
}

#[repr(C)]
pub struct UField {
    base: UObject,
    pub Next: *const UField,
}

impl_deref! { UField as UObject }

#[repr(C)]
pub struct FStructBaseChain {
    StructBaseChainArray: *const *const FStructBaseChain,
    NumStructBasesInChainMinusOne: i32,
}

impl FStructBaseChain {
    unsafe fn is(&self, parent: *const Self) -> bool {
        let parent_index = (*parent).NumStructBasesInChainMinusOne;
        let child_index = self.NumStructBasesInChainMinusOne;
        parent_index <= child_index
            && *self.StructBaseChainArray.add(parent_index as usize) == parent
    }
}

#[repr(C)]
pub struct UStruct {
    base: UField,
    struct_base_chain: FStructBaseChain,
    pub SuperStruct: *mut UStruct,
    pub Children: *const UField,
    pub ChildProperties: *const FField,
    pub PropertiesSize: i32,
    pub MinAlignment: i32,
    pad1: [u8; 80],
}

impl UStruct {
    pub unsafe fn is(&self, parent: *const Self) -> bool {
        self.struct_base_chain.is(&(*parent).struct_base_chain)
    }
}

impl_deref! { UStruct as UField }

#[repr(C)]
pub struct UClass {
    base: UStruct,
    pad0: [u8; 28],
    pub ClassFlags: EClassFlags,
    pub ClassCastFlags: EClassCastFlags,
    pad1: [u8; 344],
}

impl_deref! { UClass as UStruct }

impl UClass {
    pub fn is_blueprint_generated(&self) -> bool {
        self.ClassFlags
            .any(EClassFlags::CLASS_CompiledFromBlueprint)
    }
}

// struct FFrame : public FOutputDevice
// TODO: fill in from UnrealEngine\Engine\Source\Runtime\CoreUObject\Public\UObject\Stack.h

#[repr(C)]
pub struct FOutputDevice {
    bSuppressEventTag: bool,
    bAutoEmitLineTerminator: bool,
}

#[repr(C)]
pub struct FFrame {
    base: FOutputDevice,

    Node: *mut UFunction,
    Object: *mut UObject,

    Code: *mut u8,
    pub Locals: *mut u8,

    MostRecentProperty: *mut c_void,
    MostRecentPropertyAddress: *mut c_void,
    FlowStack: crate::TArray<u32>,
    PreviousFrame: *mut c_void,
    OutParms: *mut c_void,
    PropertyChainForCompiledIn: *mut c_void,
    CurrentNativeFunction: *mut c_void,
    bArrayContextFailed: bool,
}

pub type FNativeFuncPtr =
    unsafe extern "C" fn(Context: *mut UObject, TheStack: *mut FFrame, Result: *mut c_void);

// 	// Scope required for scoped script stats.
// 	{
// 		uint8* Frame = NULL;
// #if USE_UBER_GRAPH_PERSISTENT_FRAME
// 		if (Function->HasAnyFunctionFlags(FUNC_UbergraphFunction))
// 		{
// 			Frame = Function->GetOuterUClassUnchecked()->GetPersistentUberGraphFrame(this, Function);
// 		}
// #endif
// 		const bool bUsePersistentFrame = (NULL != Frame);
// 		if (!bUsePersistentFrame)
// 		{
// 			Frame = (uint8*)FMemory_Alloca(Function->PropertiesSize);
// 			// zero the local property memory
// 			FMemory::Memzero(Frame + Function->ParmsSize, Function->PropertiesSize - Function->ParmsSize);
// 		}

// 		// initialize the parameter properties
// 		FMemory::Memcpy(Frame, Parms, Function->ParmsSize);

// 		// Create a new local execution stack.
// 		FFrame NewStack(this, Function, Frame, NULL, Function->ChildProperties);

// 		checkSlow(NewStack.Locals || Function->ParmsSize == 0);

// inline FFrame::FFrame( UObject* InObject, UFunction* InNode, void* InLocals, FFrame* InPreviousFrame, FField* InPropertyChainForCompiledIn )
// 	: Node(InNode)
// 	, Object(InObject)
// 	, Code(InNode->Script.GetData())
// 	, Locals((uint8*)InLocals)
// 	, MostRecentProperty(NULL)
// 	, MostRecentPropertyAddress(NULL)
// 	, PreviousFrame(InPreviousFrame)
// 	, OutParms(NULL)
// 	, PropertyChainForCompiledIn(InPropertyChainForCompiledIn)
// 	, CurrentNativeFunction(NULL)
// 	, bArrayContextFailed(false)
// {
// #if DO_BLUEPRINT_GUARD
// 	FBlueprintExceptionTracker::Get().ScriptStack.Push(this);
// #endif
// }

#[repr(C)]
pub struct UFunction {
    base: UStruct,
    pub FunctionFlags: EFunctionFlags,
    NumParms: u8,
    ParmsSize: u16,
    ReturnValueOffset: u16,
    RPCId: u16,
    RPCResponseId: u16,
    FirstPropertyToInit: *const c_void,
    EventGraphFunction: *const UFunction,
    EventGraphCallOffset: i32,
    pub seen_count: u32,
    pub Func: FNativeFuncPtr,
}

#[repr(transparent)]
pub struct EFunctionFlags(u32);

impl EFunctionFlags {
    pub const FUNC_Final: Self = Self(0x1);
    pub const FUNC_RequiredAPI: Self = Self(0x2);
    pub const FUNC_BlueprintAuthorityOnly: Self = Self(0x4);
    pub const FUNC_BlueprintCosmetic: Self = Self(0x8);
    pub const FUNC_Net: Self = Self(0x40);
    pub const FUNC_NetReliable: Self = Self(0x80);
    pub const FUNC_NetRequest: Self = Self(0x100);
    pub const FUNC_Exec: Self = Self(0x200);
    pub const FUNC_Native: Self = Self(0x400);
    pub const FUNC_Event: Self = Self(0x800);
    pub const FUNC_NetResponse: Self = Self(0x1000);
    pub const FUNC_Static: Self = Self(0x2000);
    pub const FUNC_NetMulticast: Self = Self(0x4000);
    pub const FUNC_UbergraphFunction: Self = Self(0x8000);
    pub const FUNC_MulticastDelegate: Self = Self(0x10000);
    pub const FUNC_Public: Self = Self(0x20000);
    pub const FUNC_Private: Self = Self(0x40000);
    pub const FUNC_Protected: Self = Self(0x80000);
    pub const FUNC_Delegate: Self = Self(0x100000);
    pub const FUNC_NetServer: Self = Self(0x200000);
    pub const FUNC_HasOutParms: Self = Self(0x400000);
    pub const FUNC_HasDefaults: Self = Self(0x800000);
    pub const FUNC_NetClient: Self = Self(0x1000000);
    pub const FUNC_DLLImport: Self = Self(0x2000000);
    pub const FUNC_BlueprintCallable: Self = Self(0x4000000);
    pub const FUNC_BlueprintEvent: Self = Self(0x8000000);
    pub const FUNC_BlueprintPure: Self = Self(0x10000000);
    pub const FUNC_EditorOnly: Self = Self(0x20000000);
    pub const FUNC_Const: Self = Self(0x40000000);
    pub const FUNC_NetValidate: Self = Self(0x80000000);
}

impl Display for EFunctionFlags {
    fn fmt(&self, f: &mut Formatter) -> Result<(), fmt::Error> {
        let flags = self.0;

        if flags & Self::FUNC_Final.0 == Self::FUNC_Final.0 {
            write!(f, "FUNC_Final, ")?;
        }

        if flags & Self::FUNC_RequiredAPI.0 == Self::FUNC_RequiredAPI.0 {
            write!(f, "FUNC_RequiredAPI, ")?;
        }

        if flags & Self::FUNC_BlueprintAuthorityOnly.0 == Self::FUNC_BlueprintAuthorityOnly.0 {
            write!(f, "FUNC_BlueprintAuthorityOnly, ")?;
        }

        if flags & Self::FUNC_BlueprintCosmetic.0 == Self::FUNC_BlueprintCosmetic.0 {
            write!(f, "FUNC_BlueprintCosmetic, ")?;
        }

        if flags & Self::FUNC_Net.0 == Self::FUNC_Net.0 {
            write!(f, "FUNC_Net, ")?;
        }

        if flags & Self::FUNC_NetReliable.0 == Self::FUNC_NetReliable.0 {
            write!(f, "FUNC_NetReliable, ")?;
        }

        if flags & Self::FUNC_NetRequest.0 == Self::FUNC_NetRequest.0 {
            write!(f, "FUNC_NetRequest, ")?;
        }

        if flags & Self::FUNC_Exec.0 == Self::FUNC_Exec.0 {
            write!(f, "FUNC_Exec, ")?;
        }

        if flags & Self::FUNC_Native.0 == Self::FUNC_Native.0 {
            write!(f, "FUNC_Native, ")?;
        }

        if flags & Self::FUNC_Event.0 == Self::FUNC_Event.0 {
            write!(f, "FUNC_Event, ")?;
        }

        if flags & Self::FUNC_NetResponse.0 == Self::FUNC_NetResponse.0 {
            write!(f, "FUNC_NetResponse, ")?;
        }

        if flags & Self::FUNC_Static.0 == Self::FUNC_Static.0 {
            write!(f, "FUNC_Static, ")?;
        }

        if flags & Self::FUNC_NetMulticast.0 == Self::FUNC_NetMulticast.0 {
            write!(f, "FUNC_NetMulticast, ")?;
        }

        if flags & Self::FUNC_UbergraphFunction.0 == Self::FUNC_UbergraphFunction.0 {
            write!(f, "FUNC_UbergraphFunction, ")?;
        }

        if flags & Self::FUNC_MulticastDelegate.0 == Self::FUNC_MulticastDelegate.0 {
            write!(f, "FUNC_MulticastDelegate, ")?;
        }

        if flags & Self::FUNC_Public.0 == Self::FUNC_Public.0 {
            write!(f, "FUNC_Public, ")?;
        }

        if flags & Self::FUNC_Private.0 == Self::FUNC_Private.0 {
            write!(f, "FUNC_Private, ")?;
        }

        if flags & Self::FUNC_Protected.0 == Self::FUNC_Protected.0 {
            write!(f, "FUNC_Protected, ")?;
        }

        if flags & Self::FUNC_Delegate.0 == Self::FUNC_Delegate.0 {
            write!(f, "FUNC_Delegate, ")?;
        }

        if flags & Self::FUNC_NetServer.0 == Self::FUNC_NetServer.0 {
            write!(f, "FUNC_NetServer, ")?;
        }

        if flags & Self::FUNC_HasOutParms.0 == Self::FUNC_HasOutParms.0 {
            write!(f, "FUNC_HasOutParms, ")?;
        }

        if flags & Self::FUNC_HasDefaults.0 == Self::FUNC_HasDefaults.0 {
            write!(f, "FUNC_HasDefaults, ")?;
        }

        if flags & Self::FUNC_NetClient.0 == Self::FUNC_NetClient.0 {
            write!(f, "FUNC_NetClient, ")?;
        }

        if flags & Self::FUNC_DLLImport.0 == Self::FUNC_DLLImport.0 {
            write!(f, "FUNC_DLLImport, ")?;
        }

        if flags & Self::FUNC_BlueprintCallable.0 == Self::FUNC_BlueprintCallable.0 {
            write!(f, "FUNC_BlueprintCallable, ")?;
        }

        if flags & Self::FUNC_BlueprintEvent.0 == Self::FUNC_BlueprintEvent.0 {
            write!(f, "FUNC_BlueprintEvent, ")?;
        }

        if flags & Self::FUNC_BlueprintPure.0 == Self::FUNC_BlueprintPure.0 {
            write!(f, "FUNC_BlueprintPure, ")?;
        }

        if flags & Self::FUNC_EditorOnly.0 == Self::FUNC_EditorOnly.0 {
            write!(f, "FUNC_EditorOnly, ")?;
        }

        if flags & Self::FUNC_Const.0 == Self::FUNC_Const.0 {
            write!(f, "FUNC_Const, ")?;
        }

        if flags & Self::FUNC_NetValidate.0 == Self::FUNC_NetValidate.0 {
            write!(f, "FUNC_NetValidate, ")?;
        }

        Ok(())
    }
}

impl_deref! { UFunction as UStruct }

#[repr(C)]
pub struct FFieldClass {
    pad0: [u8; 8],
    pub Id: EClassCastFlags,
    pub CastFlags: EClassCastFlags,
    pad1: [u8; 40],
}

#[repr(C)]
pub struct FField {
    vtable: usize,
    pub ClassPrivate: *const FFieldClass,
    pad0: [u8; 16],
    pub Next: *const FField,
    pub NamePrivate: FName,
    pub FlagsPrivate: u32,
    pad1: [u8; 4],
}

impl FField {
    pub unsafe fn name(&self) -> &str {
        self.NamePrivate.text()
    }
}

#[derive(Copy, Clone, PartialEq, Eq)]
#[repr(transparent)]
pub struct EClassCastFlags(pub u64);

impl EClassCastFlags {
    pub const CASTCLASS_UField: Self = Self(0x1);
    pub const CASTCLASS_FInt8Property: Self = Self(0x2);
    pub const CASTCLASS_UEnum: Self = Self(0x4);
    pub const CASTCLASS_UStruct: Self = Self(0x8);
    pub const CASTCLASS_UScriptStruct: Self = Self(0x10);
    pub const CASTCLASS_UClass: Self = Self(0x20);
    pub const CASTCLASS_FByteProperty: Self = Self(0x40);
    pub const CASTCLASS_FIntProperty: Self = Self(0x80);
    pub const CASTCLASS_FFloatProperty: Self = Self(0x100);
    pub const CASTCLASS_FUInt64Property: Self = Self(0x200);
    pub const CASTCLASS_FClassProperty: Self = Self(0x400);
    pub const CASTCLASS_FUInt32Property: Self = Self(0x800);
    pub const CASTCLASS_FInterfaceProperty: Self = Self(0x1000);
    pub const CASTCLASS_FNameProperty: Self = Self(0x2000);
    pub const CASTCLASS_FStrProperty: Self = Self(0x4000);
    pub const CASTCLASS_FProperty: Self = Self(0x8000);
    pub const CASTCLASS_FObjectProperty: Self = Self(0x10000);
    pub const CASTCLASS_FBoolProperty: Self = Self(0x20000);
    pub const CASTCLASS_FUInt16Property: Self = Self(0x40000);
    pub const CASTCLASS_UFunction: Self = Self(0x80000);
    pub const CASTCLASS_FStructProperty: Self = Self(0x100000);
    pub const CASTCLASS_FArrayProperty: Self = Self(0x200000);
    pub const CASTCLASS_FInt64Property: Self = Self(0x400000);
    pub const CASTCLASS_FDelegateProperty: Self = Self(0x800000);
    pub const CASTCLASS_FNumericProperty: Self = Self(0x1000000);
    pub const CASTCLASS_FMulticastDelegateProperty: Self = Self(0x2000000);
    pub const CASTCLASS_FObjectPropertyBase: Self = Self(0x4000000);
    pub const CASTCLASS_FWeakObjectProperty: Self = Self(0x8000000);
    pub const CASTCLASS_FLazyObjectProperty: Self = Self(0x10000000);
    pub const CASTCLASS_FSoftObjectProperty: Self = Self(0x20000000);
    pub const CASTCLASS_FTextProperty: Self = Self(0x40000000);
    pub const CASTCLASS_FInt16Property: Self = Self(0x80000000);
    pub const CASTCLASS_FDoubleProperty: Self = Self(0x100000000);
    pub const CASTCLASS_FSoftClassProperty: Self = Self(0x200000000);
    pub const CASTCLASS_UPackage: Self = Self(0x400000000);
    pub const CASTCLASS_ULevel: Self = Self(0x800000000);
    pub const CASTCLASS_AActor: Self = Self(0x1000000000);
    pub const CASTCLASS_APlayerController: Self = Self(0x2000000000);
    pub const CASTCLASS_APawn: Self = Self(0x4000000000);
    pub const CASTCLASS_USceneComponent: Self = Self(0x8000000000);
    pub const CASTCLASS_UPrimitiveComponent: Self = Self(0x10000000000);
    pub const CASTCLASS_USkinnedMeshComponent: Self = Self(0x20000000000);
    pub const CASTCLASS_USkeletalMeshComponent: Self = Self(0x40000000000);
    pub const CASTCLASS_UBlueprint: Self = Self(0x80000000000);
    pub const CASTCLASS_UDelegateFunction: Self = Self(0x100000000000);
    pub const CASTCLASS_UStaticMeshComponent: Self = Self(0x200000000000);
    pub const CASTCLASS_FMapProperty: Self = Self(0x400000000000);
    pub const CASTCLASS_FSetProperty: Self = Self(0x800000000000);
    pub const CASTCLASS_FEnumProperty: Self = Self(0x1000000000000);
    pub const CASTCLASS_USparseDelegateFunction: Self = Self(0x2000000000000);
    pub const CASTCLASS_FMulticastInlineDelegateProperty: Self = Self(0x4000000000000);
    pub const CASTCLASS_FMulticastSparseDelegateProperty: Self = Self(0x8000000000000);
    pub const CASTCLASS_FFieldPathProperty: Self = Self(0x10000000000000);

    pub fn any(&self, Self(flags): Self) -> bool {
        self.0 & flags != 0
    }
}

impl BitOr for EClassCastFlags {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self::Output {
        Self(self.0 | rhs.0)
    }
}

#[derive(Copy, Clone)]
#[repr(transparent)]
pub struct EClassFlags(u32);

impl EClassFlags {
    pub const CLASS_CompiledFromBlueprint: Self = Self(0x40000);

    pub fn any(&self, Self(flags): Self) -> bool {
        self.0 & flags != 0
    }
}

#[repr(C)]
pub struct UPackage {
    base: UObject,
    unneeded_0: [u8; 56],
    pub PIEInstanceID: i32,
    unneeded_1: [u8; 60],
}

impl UPackage {
    pub fn short_name(&self) -> &str {
        let name = unsafe { self.base.name() }.as_bytes();
        let name = ReverseSplitIterator::new(name, b'/')
            .next()
            .unwrap_or(b"UPackage::short_name(): empty object name");

        // SAFETY: We started with an ASCII string (`self.base.name()`) and
        // split on an ASCII delimiter (`/`). Therefore, we still have a valid
        // ASCII string after the split. Since ASCII is a subset of UTF-8, the
        // bytes in `name` are valid UTF-8.
        unsafe { str::from_utf8_unchecked(name) }
    }
}
