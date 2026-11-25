//! The words a drawing prints that SolidCraft gives it (#19884): its title blocks' labels, its
//! tables' headings, its view labels' words and a built-in template's note and document type, saved
//! as built-in messages ([`PaperText`]) and printed in the drawing's language wherever a sheet is
//! laid out, so a change of the language reprints them. Text someone typed prints as typed, in every
//! language.
//!
//! - [`resolve`] is the one rule every layout calls: typed text as it is; a message in the
//!   drawing's language, from the catalog the host installed (exact tag, then its base language),
//!   in logical order, as paper sets right-to-left text itself.
//! - Without that catalog's word (the CLI and MCP install none yet) a message prints its English
//!   ([`MESSAGES`]), which a test keeps equal to the catalog's, as `plugin::messages` does.
//! - [`in_english`] reads a title block as English prints it, so a block a drawing saved before
//!   this change, its labels typed in English, still counts as the built-in one.
//! - [`default_name`] is the one rule for the names SolidCraft gives a design and its parts
//!   (`Untitled`, `Sketch1`, `Body2`, `Component3`), which the screen and paper both ask: the
//!   screen in its display order, paper ([`given_name`]) in reading order. So a design never named
//!   prints "Sans titre" in a French drawing's Title cell and parts list, as its tab says.
//!
//! **Cost:** a message is a search of the installed catalogs' list (about 40) and a map lookup, about
//! 40 a sheet, each time a sheet's text is laid out, never per frame. A name is three prefix checks,
//! and a lookup only when it is a default name.

use std::borrow::Cow;

use solidcraft_geom::cell::PaperText;
use solidcraft_geom::title_block::TitleBlock;

/// The built-in paper messages and their English text: the labels of the built-in title blocks
/// (`drawing.paper.cell.<cell>`), the headings of a new table (`drawing.paper.heading.<column>`), the
/// words a built-in template gives its cells (`drawing.paper.template.*`) and the words of a new view
/// label (`drawing.paper.view_label.*`).
pub const MESSAGES: &[(&str, &str)] = &[
    ("drawing.paper.cell.approved_by", "Approved by"),
    ("drawing.paper.cell.created_by", "Created by"),
    ("drawing.paper.cell.date_of_issue", "Date of issue"),
    ("drawing.paper.cell.department", "Department"),
    ("drawing.paper.cell.document_status", "Document status"),
    ("drawing.paper.cell.document_type", "Document type"),
    ("drawing.paper.cell.finish", "Finish"),
    ("drawing.paper.cell.general_tolerances", "General tolerances"),
    ("drawing.paper.cell.identification_number", "Identification number"),
    ("drawing.paper.cell.language", "Language"),
    ("drawing.paper.cell.legal_owner", "Legal owner"),
    ("drawing.paper.cell.mass", "Mass"),
    ("drawing.paper.cell.material", "Material"),
    ("drawing.paper.cell.projection", "Projection"),
    ("drawing.paper.cell.revision", "Revision"),
    ("drawing.paper.cell.scale", "Scale"),
    ("drawing.paper.cell.sheet", "Sheet"),
    ("drawing.paper.cell.title", "Title"),
    ("drawing.paper.heading.ballooned", "BALLOONED"),
    ("drawing.paper.heading.checker", "CHECKED"),
    ("drawing.paper.heading.date", "DATE"),
    ("drawing.paper.heading.description", "DESCRIPTION"),
    ("drawing.paper.heading.designer", "DRAWN"),
    ("drawing.paper.heading.item", "ITEM"),
    ("drawing.paper.heading.mass", "MASS"),
    ("drawing.paper.heading.material", "MATERIAL"),
    ("drawing.paper.heading.name", "NAME"),
    ("drawing.paper.heading.quantity", "QTY"),
    ("drawing.paper.heading.revision", "REV"),
    ("drawing.paper.template.assembly_note", "Unless stated: assembled positions ±0.5 mm, angles ±1°. Part tolerances are on the part drawings."),
    ("drawing.paper.template.assembly_type", "Assembly drawing"),
    ("drawing.paper.template.part_note", "Unless stated: linear ±0.2 mm, angles ±0.5°. Break sharp edges 0.2 mm."),
    ("drawing.paper.template.part_type", "Part drawing"),
    ("drawing.paper.view_label.detail", "DETAIL"),
    ("drawing.paper.view_label.scale", "SCALE"),
    ("drawing.paper.view_label.view", "VIEW"),
];

/// What `text` prints in a drawing written in `language` (its BCP 47 tag): typed text as typed; a
/// built-in message in that language, else in English, else its id, which no catalog has.
pub fn resolve<'t>(language: &str, text: &'t PaperText) -> Cow<'t, str> {
    match (text.typed_text(), text.message()) {
        (Some(typed), _) => Cow::Borrowed(typed),
        (None, Some(id)) => Cow::Borrowed(translated(language, id).or_else(|| english(id)).unwrap_or(id)),
        (None, None) => Cow::Borrowed(""),
    }
}

/// Message `id`'s English, as the CLI prints it.
pub fn english(id: &str) -> Option<&'static str> {
    MESSAGES.iter().find(|(known, _)| *known == id).map(|(_, text)| *text)
}

/// Message `id` in the installed catalog for `language` or, failing that, its base language
/// (`fr-CA`, then `fr`), in reading order; none when neither catalog has it.
fn translated(language: &str, id: &str) -> Option<&'static str> {
    let base = language.split(['-', '_']).next().unwrap_or(language);
    [language, base].into_iter().find_map(|tag| {
        let catalog = solidcraft_ui_text::locales().iter().find(|catalog| catalog.id.eq_ignore_ascii_case(tag))?;
        catalog.messages().get(id).map(String::as_str)
    })
}

/// `text` in a language when it is a name SolidCraft gives by default, else none: `Untitled`, a new
/// design's name, as `ui.untitled`, and `Sketch1`, `Body2` and `Component3` as their kind's message
/// with the number. `message` reads a message in that language (the screen's display order, or
/// paper's reading order, [`given_name`]); none when the language lacks it leaves `text`, which is
/// its English, as it is. Only the whole name, exactly as SolidCraft gives it: "Untitled bracket",
/// "untitled" and another language's word for it ("Sans titre") are names someone typed. A name typed
/// as exactly "Untitled" reads as the default, as a design under it counts as never named.
pub fn default_name<'m>(text: &str, message: impl FnOnce(&'static str) -> Option<&'m str>) -> Option<String> {
    let (untitled, id) = UNTITLED;
    if text == untitled {
        return message(id).map(str::to_string);
    }
    for (prefix, id) in NUMBERED_NAMES {
        if let Some(number) = text.strip_prefix(prefix)
            && !number.is_empty()
            && number.chars().all(|c| c.is_ascii_digit())
        {
            // The number is digits only.
            return message(id).map(|template| template.replace(NUMBER, number));
        }
    }
    None
}

/// A new design's name and the message that says it in a language.
pub(crate) const UNTITLED: (&str, &str) = ("Untitled", "ui.untitled");
/// The default names SolidCraft numbers (`Sketch1`, `Body2`, `Component3`): the name before its
/// number, and the message that says it in a language, with [`NUMBER`] where the number goes.
const NUMBERED_NAMES: [(&str, &str); 3] = [("Sketch", "browser.sketch_name"), ("Body", "browser.body_name"), ("Component", "browser.component_name")];
/// The one placeholder of a [`NUMBERED_NAMES`] message.
const NUMBER: &str = "{number}";

/// A name of the design or a part of it as a drawing written in `language` prints it: a default
/// name ([`default_name`]) in that language, in reading order, as the screen shows it; any other
/// name as given.
pub fn given_name<'t>(language: &str, text: &'t str) -> Cow<'t, str> {
    default_name(text, |id| translated(language, id)).map_or(Cow::Borrowed(text), Cow::Owned)
}

/// `block` as it prints in English, each label typed: two blocks that print the same in English are
/// the same block, whichever of them types its labels and whichever names built-in messages.
pub fn in_english(block: &TitleBlock) -> TitleBlock {
    let mut english = block.clone();
    for label in english.cells.iter_mut().filter_map(|cell| cell.label.as_mut()) {
        label.text = PaperText::typed(resolve("en", &label.text));
    }
    english
}

/// The pseudo-language of the engine's test catalogs (G2, #19884): every paper message and default
/// name ([`default_name`]) is its English between [`PSEUDO_OPEN`] and [`PSEUDO_CLOSE`], so a word on
/// paper that isn't wrapped didn't come through [`resolve`] or [`given_name`]. `qps-ploc` is a pseudo-locale tag (`qps` is in BCP 47's
/// private-use range).
#[cfg(test)]
pub(crate) const PSEUDO: &str = "qps-ploc";
/// The marks round a pseudo-language word: Liberation Sans has them, so a wrapped word stays one run
/// in the built-in font with no font to fall back to (it has no `⟦` or `⟧`).
#[cfg(test)]
pub(crate) const PSEUDO_OPEN: char = '‹';
#[cfg(test)]
pub(crate) const PSEUDO_CLOSE: char = '›';

/// Catalogs for the engine's tests, which no host installs: English; French and German with the
/// paper words and default names they read; Chinese and Traditional Chinese with a paper word;
/// Hebrew, right to left, with one name; and [`PSEUDO`] with every one of [`MESSAGES`] and the
/// default names, made from them. One list, installed once per process, so tests running side by side agree; the system
/// language is pinned to English first, as the app's own tests pin it.
#[cfg(test)]
pub(crate) fn install_test_catalogs() {
    use std::collections::BTreeMap;
    use std::sync::OnceLock;

    use solidcraft_ui_text::Catalog;
    static ENGLISH: Catalog = Catalog::new("en", "English", "{}");
    static FRENCH: Catalog = Catalog::new(
        "fr",
        "Français",
        r#"{"drawing.paper.cell.title": "Titre", "drawing.paper.cell.legal_owner": "Propriétaire légal",
            "drawing.paper.heading.item": "REP", "drawing.paper.heading.quantity": "QTÉ",
            "drawing.paper.view_label.view": "VUE", "drawing.paper.view_label.detail": "DÉTAIL",
            "drawing.paper.view_label.scale": "ÉCHELLE", "ui.untitled": "Sans titre",
            "browser.component_name": "Composant{number}",
            "drawing.paper.template.part_note": "Sauf indication contraire : linéaire ±0.2 mm, angles ±0.5°. Casser les arêtes vives 0.2 mm.",
            "drawing.paper.template.assembly_note": "Sauf indication contraire : positions d'assemblage ±0.5 mm, angles ±1°. Les tolérances des pièces figurent sur les plans de pièces.",
            "drawing.paper.template.part_type": "Plan de pièce", "drawing.paper.template.assembly_type": "Plan d'assemblage"}"#,
    );
    static GERMAN: Catalog = Catalog::new(
        "de",
        "Deutsch",
        r#"{"drawing.paper.cell.title": "Titel", "drawing.paper.cell.legal_owner": "Rechtlicher Eigentümer",
            "drawing.paper.heading.item": "POS.", "drawing.paper.heading.quantity": "MENGE",
            "drawing.paper.view_label.view": "ANSICHT", "drawing.paper.view_label.detail": "EINZELHEIT",
            "drawing.paper.view_label.scale": "MASSSTAB", "ui.untitled": "Ohne Titel"}"#,
    );
    static HEBREW: Catalog = Catalog::new("he", "עברית", r#"{"ui.untitled": "ללא שם"}"#);
    static CHINESE: Catalog = Catalog::new("zh", "中文", r#"{"drawing.paper.view_label.view": "视图"}"#);
    static CHINESE_TRADITIONAL: Catalog = Catalog::new("zh-TW", "繁體中文", r#"{"drawing.paper.view_label.view": "視圖"}"#);
    static WRAPPED: OnceLock<Catalog> = OnceLock::new();
    static LOCALES: OnceLock<[&Catalog; 7]> = OnceLock::new();
    let locales = LOCALES.get_or_init(|| {
        let numbered = NUMBERED_NAMES.map(|(prefix, id)| (id, format!("{prefix}{NUMBER}")));
        let english = MESSAGES.iter().map(|(id, english)| (*id, english.to_string())).chain([(UNTITLED.1, UNTITLED.0.to_string())]).chain(numbered);
        let wrapped: BTreeMap<&str, String> = english.map(|(id, english)| (id, format!("{PSEUDO_OPEN}{english}{PSEUDO_CLOSE}"))).collect();
        let json = serde_json::to_string(&wrapped).unwrap_or_default();
        let pseudo = WRAPPED.get_or_init(|| Catalog::new(PSEUDO, "Pseudo", Box::leak(json.into_boxed_str())));
        [&ENGLISH, &FRENCH, &GERMAN, &HEBREW, &CHINESE, &CHINESE_TRADITIONAL, pseudo]
    });
    solidcraft_ui_text::pin_system_language();
    solidcraft_ui_text::install(locales);
}

#[cfg(test)]
mod tests {
    use solidcraft_doc::drawing::SheetComponent;
    use solidcraft_doc::drawing::table::{Kind, MassFormat, PartsList};
    use solidcraft_doc::drawing::template::{BUILT_INS, built_in};
    use solidcraft_doc::drawing::view::{DETAIL_WORDS, ViewLabel};
    use solidcraft_geom::title_block::{compact_block, default_block};

    use super::*;

    /// Every built-in message the built-in blocks, a new table, a new view label and the built-in
    /// templates' cells name has its English here (G1): a message without it would print its id in
    /// the CLI. The app's tests keep this English equal to the catalog's and check every language has
    /// the words.
    #[test]
    fn every_built_in_paper_word_has_its_english() {
        let list = Kind::PartsList(PartsList {
            of: SheetComponent { source: 1, component: 0 },
            view: Default::default(),
            group: Default::default(),
            fold: false,
            purchased_contents: false,
            mass: MassFormat::default(),
        });
        let label = ViewLabel::given("A".into());
        let mut named: Vec<PaperText> = Vec::new();
        named.extend(default_block().cells.into_iter().chain(compact_block().cells).filter_map(|cell| cell.label).map(|label| label.text));
        named.extend([list, Kind::Revision].iter().flat_map(|kind| kind.default_table().columns).map(|column| column.heading));
        named.extend([label.words, label.scale_words, PaperText::built_in(DETAIL_WORDS)]);
        // Each template's note and document type, over its first sheet's cells.
        for name in BUILT_INS {
            let template = built_in(name).unwrap_or_else(|| panic!("no built-in {name}")).unwrap();
            named.extend(template.sheets.iter().flat_map(|sheet| sheet.title_block_overrides.values().cloned()));
        }
        assert_eq!(named.len(), 18 + 12 + 12 + 3 + 10 * 2);
        for text in &named {
            let id = text.message().unwrap_or_else(|| panic!("{text:?} is typed: a built-in word is a message"));
            assert!(english(id).is_some(), "{id} has no English");
        }
        // And nothing here that nothing names.
        let ids: std::collections::BTreeSet<&str> = named.iter().filter_map(PaperText::message).collect();
        assert_eq!(ids.len(), MESSAGES.len());
        assert!(MESSAGES.windows(2).all(|pair| pair[0].0 < pair[1].0), "sorted by id, once each");
    }

    /// Typed text prints as typed in every language, even when it reads like a message's id; a
    /// message prints its English with no catalog for the language, and an id nothing knows prints
    /// as itself. A block saved with English labels typed is the built-in block in English; one
    /// label typed otherwise isn't.
    #[test]
    fn typed_text_stays_and_a_message_falls_back_to_english() {
        for language in ["en", "xx", "fr-CA", ""] {
            assert_eq!(resolve(language, &PaperText::typed("Werkstoff")), "Werkstoff");
            assert_eq!(resolve(language, &PaperText::typed("drawing.paper.cell.title")), "drawing.paper.cell.title");
        }
        assert_eq!(resolve("xx", &PaperText::built_in("drawing.paper.heading.quantity")), "QTY");
        assert_eq!(resolve("en", &PaperText::built_in("drawing.paper.no_such_word")), "drawing.paper.no_such_word");
        let mut saved = default_block();
        for label in saved.cells.iter_mut().filter_map(|cell| cell.label.as_mut()) {
            label.text = PaperText::typed(english(label.text.message().unwrap_or_default()).unwrap_or_default());
        }
        assert_ne!(saved, default_block());
        assert_eq!(in_english(&saved), in_english(&default_block()));
        if let Some(label) = saved.cells.first_mut().and_then(|cell| cell.label.as_mut()) {
            label.text = PaperText::typed("Part name");
        }
        assert_ne!(in_english(&saved), in_english(&default_block()));
    }

    /// With catalogs installed, a message prints the drawing's language, by its exact tag or its base
    /// language; a word that language lacks prints English.
    #[test]
    fn a_message_prints_in_the_drawings_language() {
        install_test_catalogs();
        let title = PaperText::built_in("drawing.paper.cell.title");
        assert_eq!([resolve("fr", &title), resolve("fr-CA", &title), resolve("DE", &title)], ["Titre", "Titre", "Titel"]);
        let view = PaperText::built_in("drawing.paper.view_label.view");
        assert_eq!([resolve("zh-TW", &view), resolve("zh", &view)], ["視圖", "视图"], "the exact tag's word, not its base language's");
        assert_eq!(resolve("fr", &PaperText::built_in("drawing.paper.cell.mass")), "Mass", "the test catalog lacks it");
        assert_eq!(resolve("ja", &title), "Title", "no catalog: English");
    }

    /// A name SolidCraft gives by default prints in the drawing's language (#19884), by its exact tag
    /// or its base language, a number in its kind's word, and right-to-left text in reading order, not
    /// the screen's display order. With no word for it, it prints as given, which is its English. Only
    /// the whole name, exactly: another language's word for Untitled, Untitled in other capitals or
    /// with more words, and a kind with no number are names someone typed, and print as typed.
    #[test]
    fn a_default_name_prints_in_the_drawings_language_and_a_typed_one_as_typed() {
        install_test_catalogs();
        assert_eq!(
            [given_name("fr", "Untitled"), given_name("fr-CA", "Untitled"), given_name("de", "Untitled")],
            ["Sans titre", "Sans titre", "Ohne Titel"]
        );
        assert_eq!(given_name("fr", "Component3"), "Composant3");
        assert_eq!(given_name("he", "Untitled"), "ללא שם", "reading order");
        for language in ["en", "ja", "xx", ""] {
            for name in ["Untitled", "Component3", "Sketch1"] {
                assert_eq!(given_name(language, name), name, "{language}: no word for it, so its English");
            }
        }
        for typed in ["Sans titre", "Ohne Titel", "untitled", "UNTITLED", "Untitled bracket", " Untitled", "Component", "Component3a", "Bracket"] {
            for language in ["fr", "de", "he", "en"] {
                assert_eq!(given_name(language, typed), typed, "{language}: `{typed}` is typed");
            }
        }
        assert_eq!(default_name("Body2", |id| (id == "browser.body_name").then_some("Corps {number}")).as_deref(), Some("Corps 2"));
        assert_eq!(default_name("Body2", |_| None), None, "a language without the word");
    }
}
