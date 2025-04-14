/// Build script for the `revmc` builtin and runtime.
fn main() {
    #[cfg(feature = "compiler")]
    revmc_build::emit();
}
