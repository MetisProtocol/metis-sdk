/// Build script for the `metis-pe` crate.
fn main() {
    #[cfg(feature = "compiler")]
    revmc_build::emit();
}
