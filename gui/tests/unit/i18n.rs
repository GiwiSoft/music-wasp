use super::*;

#[test]
fn english_catalogue_and_fallback() {
    set_language(Language::English);
    assert_eq!(tr("settings.title"), "Settings");
    set_language(Language::French);
    assert_eq!(tr("settings.title"), "Paramètres");
    assert_eq!(tr("ui.definitely_missing_key"), "ui.definitely_missing_key");
    set_language(Language::English);
}

#[test]
fn every_language_has_a_native_label_and_catalogue() {
    let english = dictionary(Language::English);
    assert!(!english.is_empty());
    for language in Language::ALL {
        assert!(!language.label().is_empty());
        assert!(!language.code().is_empty());
        if language != Language::English {
            let catalogue = dictionary(language);
            assert!(!catalogue.is_empty(), "{language:?} has no translations");
            for key in catalogue.keys() {
                assert!(
                    english.contains_key(key),
                    "{language:?} has unknown key {key}"
                );
            }
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
