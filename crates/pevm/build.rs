/// Build script for the `pevm` crate.
fn main() {
    #[cfg(feature = "compiler")]
    revmc_build::emit();
}
