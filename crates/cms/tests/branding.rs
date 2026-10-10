//! Product labels change without changing stable profile ids or legacy preference lookup.
use photocraft_cms::{Builtin, Profile};

#[test]
fn builtin_profiles_use_prism_and_keep_legacy_lookup() {
    for builtin in Builtin::ALL {
        let description = builtin.description();
        assert!(!description.to_ascii_lowercase().contains("photocraft"));
        assert_eq!(Builtin::from_id(builtin.id()), Some(builtin));
        assert_eq!(Builtin::from_id(description), Some(builtin));
        assert_eq!(Builtin::from_id(&description.replace("Prism", "Photocraft")), Some(builtin));
        assert_eq!(Builtin::from_id(&description.replace("Prism", "Photocraft").to_ascii_lowercase()), Some(builtin));
        assert_eq!(builtin.profile().description, description);
        let parsed = Profile::parse(&builtin.profile().to_bytes()).unwrap();
        assert_eq!(parsed.description, description);
    }
}
