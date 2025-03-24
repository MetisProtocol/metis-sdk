use std::sync::Once;
use tokio::runtime::Runtime;

static mut RUNTIME: Option<Runtime> = None;
static RUNTIME_INIT: Once = Once::new();

/// Makes sure only a single runtime thread is alive throughout the program lifetime.
#[allow(static_mut_refs)]
#[allow(unused)]
pub(crate) fn get_runtime() -> &'static Runtime {
    unsafe {
        RUNTIME_INIT.call_once(|| {
            RUNTIME = Some(Runtime::new().unwrap());
        });
        RUNTIME.as_ref().unwrap()
    }
}
