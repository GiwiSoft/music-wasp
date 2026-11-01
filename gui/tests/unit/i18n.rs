use super::*;

#[test]
fn english_is_the_identity_and_fallback() {
    set_language(Language::English);
    assert_eq!(tr("Settings"), "Settings");
    set_language(Language::French);
    assert_eq!(tr("Settings"), "Paramètres");
    assert_eq!(tr("not translated at all"), "not translated at all");
    set_language(Language::English);
}

#[test]
fn every_language_has_a_native_label_and_catalogue() {
    for language in Language::ALL {
        assert!(!language.label().is_empty());
        assert!(!language.code().is_empty());
        if language != Language::English {
            assert!(
                !dictionary(language).is_empty(),
                "{language:?} has no translations"
            );
        }
    }
    assert_eq!(
        serde_json::from_str::<Language>("\"fr\"").unwrap(),
        Language::French
    );
    assert_eq!(
        serde_json::from_str::<Language>("\"de\"").unwrap(),
        Language::German
    );
    assert_eq!(
        serde_json::from_str::<Language>("\"es\"").unwrap(),
        Language::Spanish
    );
}
