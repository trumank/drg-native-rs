use crate::hooks::OUTLINE_COMPONENT;
use common::UObject;
use sdk::Engine::Pawn;
use sdk::FSD::OutlineComponent;

pub unsafe fn set_outline(pawn: *mut Pawn) {
    for &component in (*pawn).BlueprintCreatedComponents.iter() {
        if (*component.cast::<UObject>()).is(OUTLINE_COMPONENT) {
            let component = component.cast::<OutlineComponent>();
            (*component).UnlockOutline();
            (*component).ToggleDefaultOutline(true);
            (*component).LockOutline();
        }
    }
}
