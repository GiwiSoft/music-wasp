use super::*;

#[test]
fn every_skin_has_a_distinct_palette() {
    let mut seen = Vec::new();
    for skin in Skin::all() {
        let palette = skin.palette();
        assert!(!seen.contains(&palette), "{:?} reuses a palette", skin);
        seen.push(palette);
    }
}

#[test]
fn applying_a_skin_updates_the_shared_palette() {
    let ctx = egui::Context::default();
    apply(&ctx, Skin::MIDNIGHT);
    assert_eq!(accent(), Skin::MIDNIGHT.palette().accent);
    assert_eq!(text(), Skin::MIDNIGHT.palette().text);

    apply(&ctx, Skin::LIGHT);
    assert_eq!(accent(), Skin::LIGHT.palette().accent);
    assert!(Skin::LIGHT.is_light());

    // Leave the default behind for any other test that paints.
    apply(&ctx, Skin::default());
}

#[test]
fn high_contrast_skin_is_dark_and_maximally_legible() {
    let palette = Skin::HIGH_CONTRAST.palette();
    assert!(!Skin::HIGH_CONTRAST.is_light());
    assert_eq!(palette.bg_dark, Color32::BLACK);
    assert_eq!(palette.text, Color32::WHITE);
    assert!(palette.accent.r() > 240);
    assert!(palette.text_dim.r() > 180);
    assert_eq!(Skin::HIGH_CONTRAST.label(), "High contrast");
    assert!(Skin::all().contains(&Skin::HIGH_CONTRAST));
}

#[test]
fn liquid_glass_themes_have_translucent_surfaces_and_backdrops() {
    for skin in [Skin::LIQUID_GLASS_DARK, Skin::LIQUID_GLASS_LIGHT] {
        assert!(skin.is_glass());
        let palette = skin.palette();
        assert!(palette.bg_panel.a() < 255, "{skin:?} panels are opaque");
        assert!(palette.bg_dark.a() < 255, "{skin:?} background is opaque");
        assert!(skin.backdrop().is_some());
        assert!(skin.label().starts_with("Liquid Glass"));
    }
    assert!(Skin::LIQUID_GLASS_DARK.backdrop().is_some());
    assert!(!Skin::WASP.is_glass());
    assert!(Skin::LIQUID_GLASS_LIGHT.is_light());
    assert!(!Skin::LIQUID_GLASS_DARK.is_light());
}

#[test]
fn winamp_classic_skin_is_flat_and_green() {
    let skin = Skin::from_id("winamp_classic");
    assert_eq!(skin.label(), "Winamp Classic");
    assert!(skin.is_square());
    assert!(skin.is_compact());
    assert!(!skin.is_glass());
    let palette = skin.palette();
    assert!(palette.accent.g() > 220);
    assert!(Skin::all().iter().any(|item| item.id() == "winamp_classic"));
}

#[test]
fn external_skin_json_parses_colours_aliases_and_icons() {
    let definitions = parse_skin_file(
        r##"[
            {
                "id": "custom",
                "label": "Custom",
                "aliases": ["c"],
                "light": true,
                "glass": true,
                "high_contrast": false,
                "palette": {
                    "accent": "#123456",
                    "bg_dark": "#01020380"
                },
                "backdrop": ["#000000", "#111111", "#222222", "#333333"],
                "icon": "logo.png"
            }
        ]"##,
        Some(Path::new("/skins")),
    )
    .unwrap();
    let skin = &definitions[0];
    assert_eq!(skin.id, "custom");
    assert_eq!(skin.label, "Custom");
    assert_eq!(skin.aliases, vec!["c"]);
    assert!(skin.light && skin.glass);
    assert_eq!(skin.palette.accent, Color32::from_rgb(0x12, 0x34, 0x56));
    assert_eq!(skin.palette.bg_dark.a(), 0x80);
    assert_eq!(
        skin.icon_path.as_deref(),
        Some(Path::new("/skins/logo.png"))
    );
    assert_eq!(
        skin.backdrop.unwrap()[1],
        Color32::from_rgb(0x11, 0x11, 0x11)
    );
}
