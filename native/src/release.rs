//! Compile-time release scope; backend operations also enforce this gate.
pub const STORE: bool = cfg!(feature = "store");
pub const EDITION: &str = if STORE { "store" } else { "portable" };
pub fn require_system_mutations() -> Result<(), String> {
    if STORE {
        Err("Store edition provides service, startup and system memory inventory only. Use Windows Settings or Windows administration tools to make system changes.".into())
    } else {
        Ok(())
    }
}
