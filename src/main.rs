//! The `hq` command. Verbs are listed in SPEC 4.2; none is wired yet — this
//! binary exists so the crate builds as what it will be.

fn main() {
    println!(
        "hq {} — see SPEC.md; no verb is implemented yet",
        env!("CARGO_PKG_VERSION")
    );
}
