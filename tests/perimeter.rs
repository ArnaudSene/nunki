//! The allowlist rules of SPEC 4.1 bis, rules 5 and 6.

use hq::harness::Role;
use hq::mission::Service;
use hq::perimeter::{PerimeterError, Profile, Sources, compute};

fn strings(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| s.to_string()).collect()
}

#[test]
fn the_coder_gets_his_stack_and_his_harness_and_nothing_else() {
    let stack = strings(&["static.crates.io", "index.crates.io"]);
    let harness = strings(&["api.anthropic.com"]);
    let services = vec![Service {
        name: "db".to_string(),
        reach: strings(&["db.internal"]),
    }];
    let coder = compute(
        Role::Coder,
        &Sources {
            stack: &stack,
            harness: &harness,
            services: &[],
            forge: &[],
        },
    )
    .unwrap();

    assert_eq!(
        coder.domains.iter().cloned().collect::<Vec<_>>(),
        vec!["api.anthropic.com", "index.crates.io", "static.crates.io"]
    );
    assert!(coder.addresses.is_empty());

    // The same sources plus the mission's services, on a system profile.
    let integrator = compute(
        Role::Integrator,
        &Sources {
            stack: &stack,
            harness: &harness,
            services: &services,
            forge: &[],
        },
    )
    .unwrap();
    assert!(integrator.domains.contains("db.internal"));
    assert!(!coder.domains.contains("db.internal"));
}

#[test]
fn a_mission_profile_carrying_services_is_refused() {
    let services = vec![Service {
        name: "db".to_string(),
        reach: strings(&["db.internal"]),
    }];
    let err = compute(
        Role::Coder,
        &Sources {
            stack: &strings(&["index.crates.io"]),
            harness: &[],
            services: &services,
            forge: &[],
        },
    )
    .unwrap_err();
    assert!(matches!(
        err,
        PerimeterError::ServicesOnMissionProfile { count: 1 }
    ));
}

#[test]
fn the_forge_is_refused_whoever_declares_it() {
    let forge = strings(&["github.com"]);
    let err = compute(
        Role::Coder,
        &Sources {
            stack: &strings(&["github.com"]),
            harness: &[],
            services: &[],
            forge: &forge,
        },
    )
    .unwrap_err();
    assert!(matches!(err, PerimeterError::Forge { .. }), "{err}");
    assert!(err.to_string().contains("the stack fragment"), "{err}");

    // A subdomain of the forge is the forge.
    let err = compute(
        Role::Security,
        &Sources {
            stack: &[],
            harness: &strings(&["API.GitHub.com."]),
            services: &[],
            forge: &forge,
        },
    )
    .unwrap_err();
    assert_eq!(
        match err {
            PerimeterError::Forge { domain, .. } => domain,
            other => panic!("expected a forge refusal, got {other}"),
        },
        "api.github.com"
    );

    // A name that merely ends the same way is not.
    compute(
        Role::Coder,
        &Sources {
            stack: &strings(&["notgithub.com"]),
            harness: &[],
            services: &[],
            forge: &forge,
        },
    )
    .expect("notgithub.com is not a subdomain of github.com");
}

#[test]
fn an_address_is_an_address_and_a_name_is_a_name() {
    let services = vec![Service {
        name: "db".to_string(),
        reach: strings(&["10.4.0.7", "10.4.0.0/24", "::1", "db.internal"]),
    }];
    let p = compute(
        Role::Integrator,
        &Sources {
            stack: &[],
            harness: &strings(&["api.anthropic.com"]),
            services: &services,
            forge: &[],
        },
    )
    .unwrap();
    assert_eq!(
        p.addresses.iter().cloned().collect::<Vec<_>>(),
        vec!["10.4.0.0/24", "10.4.0.7", "::1"]
    );
    assert_eq!(
        p.domains.iter().cloned().collect::<Vec<_>>(),
        vec!["api.anthropic.com", "db.internal"]
    );
}

#[test]
fn names_are_deduplicated_and_sorted_so_a_diff_means_a_change() {
    let a = compute(
        Role::Coder,
        &Sources {
            stack: &strings(&["b.example", "a.example", "B.example."]),
            harness: &strings(&["a.example"]),
            services: &[],
            forge: &[],
        },
    )
    .unwrap();
    assert_eq!(
        a.domains.iter().cloned().collect::<Vec<_>>(),
        vec!["a.example", "b.example"]
    );
}

#[test]
fn an_empty_entry_is_an_error_not_a_silent_hole() {
    let err = compute(
        Role::Coder,
        &Sources {
            stack: &strings(&["  "]),
            harness: &[],
            services: &[],
            forge: &[],
        },
    )
    .unwrap_err();
    assert!(matches!(err, PerimeterError::Empty { .. }), "{err}");
}

#[test]
fn profiles_follow_roles() {
    assert_eq!(Profile::of(Role::Coder), Profile::Mission);
    assert_eq!(Profile::of(Role::Integrator), Profile::System);
    assert_eq!(Profile::of(Role::Security), Profile::System);
}
