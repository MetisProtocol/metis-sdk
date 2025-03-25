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

#[cfg(feature = "compiler")]
static mut CONTEXT: Option<crate::CompilerContext> = None;
#[cfg(feature = "compiler")]
static CONTEXT_INIT: Once = Once::new();

#[allow(static_mut_refs)]
#[allow(unused)]
#[cfg(feature = "compiler")]
pub(crate) fn get_compiler_context() -> &'static crate::CompilerContext {
    unsafe {
        CONTEXT_INIT.call_once(|| {
            CONTEXT = Some(crate::CompilerContext::create());
        });
        CONTEXT.as_ref().unwrap()
    }
}
