/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

use std::sync::Arc;

use dwrote::{Font, FontCollection, FontStretch, FontStyle};
use fonts_traits::LocalFontIdentifier;
use servo_base::text::{UnicodeBlock, UnicodeBlockMethod, unicode_plane};
use style::values::computed::font::GenericFontFamily;
use style::values::computed::{FontStyle as StyleFontStyle, FontWeight as StyleFontWeight};
use style::values::specified::font::FontWidthKeyword;

use crate::{
    EmojiPresentationPreference, FallbackFontSelectionOptions, FontIdentifier, FontTemplate,
    FontTemplateDescriptor, LowercaseFontFamilyName,
};

/// RustWolf: with fingerprint protection on (`RUSTWOLF_FONT_ALLOWLIST`), pages only see the fonts
/// every Windows 10 / 11 PC has. The extra fonts installed on a PC are one of the strongest
/// fingerprints; this hides them (web fonts still work).
const RUSTWOLF_STANDARD_FONTS: &[&str] = &[
    "arial",
    "arial black",
    "bahnschrift",
    "calibri",
    "calibri light",
    "cambria",
    "cambria math",
    "candara",
    "comic sans ms",
    "consolas",
    "constantia",
    "corbel",
    "courier new",
    "ebrima",
    "franklin gothic medium",
    "gabriola",
    "gadugi",
    "georgia",
    "hololens mdl2 assets",
    "impact",
    "ink free",
    "javanese text",
    "leelawadee ui",
    "leelawadee ui semilight",
    "lucida console",
    "lucida sans unicode",
    "malgun gothic",
    "malgun gothic semilight",
    "marlett",
    "microsoft himalaya",
    "microsoft jhenghei",
    "microsoft jhenghei ui",
    "microsoft new tai lue",
    "microsoft phagspa",
    "microsoft sans serif",
    "microsoft tai le",
    "microsoft yahei",
    "microsoft yahei ui",
    "microsoft yi baiti",
    "mingliu-extb",
    "pmingliu-extb",
    "mingliu_hkscs-extb",
    "mongolian baiti",
    "ms gothic",
    "ms pgothic",
    "ms ui gothic",
    "mv boli",
    "myanmar text",
    "nirmala ui",
    "nirmala ui semilight",
    "palatino linotype",
    "segoe mdl2 assets",
    "segoe fluent icons",
    "segoe print",
    "segoe script",
    "segoe ui",
    "segoe ui black",
    "segoe ui emoji",
    "segoe ui historic",
    "segoe ui light",
    "segoe ui semibold",
    "segoe ui semilight",
    "segoe ui symbol",
    "segoe ui variable",
    "simsun",
    "nsimsun",
    "simsun-extb",
    "sitka banner",
    "sitka display",
    "sitka heading",
    "sitka small",
    "sitka subheading",
    "sitka text",
    "sylfaen",
    "symbol",
    "tahoma",
    "times new roman",
    "trebuchet ms",
    "verdana",
    "webdings",
    "wingdings",
    "yu gothic",
    "yu gothic ui",
    "yu gothic ui semibold",
    "yu gothic ui light",
    "yu gothic ui semilight",
    "yu gothic light",
    "yu gothic medium",
    "aparajita",
    "arabic typesetting",
    "estrangelo edessa",
    "euphemia",
    "khmer ui",
    "lao ui",
    "meiryo",
    "microsoft uighur",
    "nyala",
    "plantagenet cherokee",
    "traditional arabic",
    "utsaah",
];

static RUSTWOLF_FONT_ALLOWLIST: std::sync::LazyLock<bool> =
    std::sync::LazyLock::new(|| std::env::var_os("RUSTWOLF_FONT_ALLOWLIST").is_some());

fn rustwolf_font_allowed(family_name: &str) -> bool {
    !*RUSTWOLF_FONT_ALLOWLIST ||
        RUSTWOLF_STANDARD_FONTS.contains(&family_name.to_lowercase().as_str())
}

pub(crate) fn for_each_available_family<F>(mut callback: F)
where
    F: FnMut(&str),
{
    let system_fc = FontCollection::system();
    for family in system_fc.families_iter() {
        if let Ok(family_name) = family.family_name() {
            if rustwolf_font_allowed(&family_name) {
                callback(&family_name);
            }
        }
    }
}

pub(crate) fn for_each_variation<F>(family_name: &str, mut callback: F)
where
    F: FnMut(FontTemplate),
{
    if !rustwolf_font_allowed(family_name) {
        return;
    }
    let system_fc = FontCollection::system();
    if let Ok(Some(family)) = system_fc.font_family_by_name(family_name) {
        let count = family.get_font_count();
        for i in 0..count {
            let Ok(font) = family.font(i) else {
                continue;
            };
            let template_descriptor = font_template_descriptor_from_font(&font);
            let local_font_identifier = LocalFontIdentifier {
                font_descriptor: Arc::new(font.to_descriptor()),
            };
            callback(FontTemplate::new(
                FontIdentifier::Local(local_font_identifier),
                template_descriptor,
                None,
            ))
        }
    }
}

// Based on gfxWindowsPlatform::GetCommonFallbackFonts() in Gecko
pub fn fallback_font_families(options: FallbackFontSelectionOptions) -> Vec<&'static str> {
    let mut families = Vec::new();
    if options.presentation_preference == EmojiPresentationPreference::Emoji {
        families.push("Segoe UI Emoji");
    }

    families.push("Arial");
    match unicode_plane(options.character) {
        // https://en.wikipedia.org/wiki/Plane_(Unicode)#Basic_Multilingual_Plane
        0 => {
            if let Some(block) = options.character.block() {
                match block {
                    UnicodeBlock::CyrillicSupplement |
                    UnicodeBlock::Armenian |
                    UnicodeBlock::Hebrew => {
                        families.push("Estrangelo Edessa");
                        families.push("Cambria");
                    },

                    UnicodeBlock::Arabic | UnicodeBlock::ArabicSupplement => {
                        families.push("Microsoft Uighur");
                    },

                    UnicodeBlock::Syriac => {
                        families.push("Estrangelo Edessa");
                    },

                    UnicodeBlock::Thaana => {
                        families.push("MV Boli");
                    },

                    UnicodeBlock::NKo => {
                        families.push("Ebrima");
                    },

                    UnicodeBlock::Devanagari | UnicodeBlock::Bengali => {
                        families.push("Nirmala UI");
                        families.push("Utsaah");
                        families.push("Aparajita");
                    },

                    UnicodeBlock::Gurmukhi |
                    UnicodeBlock::Gujarati |
                    UnicodeBlock::Oriya |
                    UnicodeBlock::Tamil |
                    UnicodeBlock::Telugu |
                    UnicodeBlock::Kannada |
                    UnicodeBlock::Malayalam |
                    UnicodeBlock::Sinhala |
                    UnicodeBlock::Lepcha |
                    UnicodeBlock::OlChiki |
                    UnicodeBlock::CyrillicExtendedC |
                    UnicodeBlock::SundaneseSupplement |
                    UnicodeBlock::VedicExtensions => {
                        families.push("Nirmala UI");
                    },

                    UnicodeBlock::Thai => {
                        families.push("Leelawadee UI");
                    },

                    UnicodeBlock::Lao => {
                        families.push("Lao UI");
                    },

                    UnicodeBlock::Myanmar |
                    UnicodeBlock::MyanmarExtendedA |
                    UnicodeBlock::MyanmarExtendedB => {
                        families.push("Myanmar Text");
                    },

                    UnicodeBlock::HangulJamo |
                    UnicodeBlock::HangulJamoExtendedA |
                    UnicodeBlock::HangulSyllables |
                    UnicodeBlock::HangulJamoExtendedB |
                    UnicodeBlock::HangulCompatibilityJamo => {
                        families.push("Malgun Gothic");
                    },

                    UnicodeBlock::Ethiopic |
                    UnicodeBlock::EthiopicSupplement |
                    UnicodeBlock::EthiopicExtended |
                    UnicodeBlock::EthiopicExtendedA => {
                        families.push("Nyala");
                    },

                    UnicodeBlock::Cherokee => {
                        families.push("Plantagenet Cherokee");
                    },

                    UnicodeBlock::UnifiedCanadianAboriginalSyllabics |
                    UnicodeBlock::UnifiedCanadianAboriginalSyllabicsExtended => {
                        families.push("Euphemia");
                        families.push("Segoe UI");
                    },

                    UnicodeBlock::Khmer | UnicodeBlock::KhmerSymbols => {
                        families.push("Khmer UI");
                        families.push("Leelawadee UI");
                    },

                    UnicodeBlock::Mongolian => {
                        families.push("Mongolian Baiti");
                    },

                    UnicodeBlock::TaiLe => {
                        families.push("Microsoft Tai Le");
                    },

                    UnicodeBlock::NewTaiLue => {
                        families.push("Microsoft New Tai Lue");
                    },

                    UnicodeBlock::Buginese |
                    UnicodeBlock::TaiTham |
                    UnicodeBlock::CombiningDiacriticalMarksExtended => {
                        families.push("Leelawadee UI");
                    },

                    UnicodeBlock::GeneralPunctuation |
                    UnicodeBlock::SuperscriptsandSubscripts |
                    UnicodeBlock::CurrencySymbols |
                    UnicodeBlock::CombiningDiacriticalMarksforSymbols |
                    UnicodeBlock::LetterlikeSymbols |
                    UnicodeBlock::NumberForms |
                    UnicodeBlock::Arrows |
                    UnicodeBlock::MathematicalOperators |
                    UnicodeBlock::MiscellaneousTechnical |
                    UnicodeBlock::ControlPictures |
                    UnicodeBlock::OpticalCharacterRecognition |
                    UnicodeBlock::EnclosedAlphanumerics |
                    UnicodeBlock::BoxDrawing |
                    UnicodeBlock::BlockElements |
                    UnicodeBlock::GeometricShapes |
                    UnicodeBlock::MiscellaneousSymbols |
                    UnicodeBlock::Dingbats |
                    UnicodeBlock::MiscellaneousMathematicalSymbolsA |
                    UnicodeBlock::SupplementalArrowsA |
                    UnicodeBlock::SupplementalArrowsB |
                    UnicodeBlock::MiscellaneousMathematicalSymbolsB |
                    UnicodeBlock::SupplementalMathematicalOperators |
                    UnicodeBlock::MiscellaneousSymbolsandArrows |
                    UnicodeBlock::Glagolitic |
                    UnicodeBlock::LatinExtendedC |
                    UnicodeBlock::Coptic => {
                        families.push("Segoe UI");
                        families.push("Segoe UI Symbol");
                        families.push("Cambria");
                        families.push("Meiryo");
                        families.push("Lucida Sans Unicode");
                        families.push("Ebrima");
                    },

                    UnicodeBlock::GeorgianSupplement |
                    UnicodeBlock::Tifinagh |
                    UnicodeBlock::CyrillicExtendedA |
                    UnicodeBlock::SupplementalPunctuation |
                    UnicodeBlock::CJKRadicalsSupplement |
                    UnicodeBlock::KangxiRadicals |
                    UnicodeBlock::IdeographicDescriptionCharacters => {
                        families.push("Segoe UI");
                        families.push("Segoe UI Symbol");
                        families.push("Meiryo");
                    },

                    UnicodeBlock::BraillePatterns => {
                        families.push("Segoe UI Symbol");
                    },

                    UnicodeBlock::CJKSymbolsandPunctuation |
                    UnicodeBlock::Hiragana |
                    UnicodeBlock::Katakana |
                    UnicodeBlock::Bopomofo |
                    UnicodeBlock::Kanbun |
                    UnicodeBlock::BopomofoExtended |
                    UnicodeBlock::CJKStrokes |
                    UnicodeBlock::KatakanaPhoneticExtensions |
                    UnicodeBlock::CJKUnifiedIdeographs => {
                        families.push("Microsoft YaHei");
                        families.push("Yu Gothic");
                    },

                    UnicodeBlock::EnclosedCJKLettersandMonths => {
                        families.push("Malgun Gothic");
                    },

                    UnicodeBlock::YijingHexagramSymbols => {
                        families.push("Segoe UI Symbol");
                    },

                    UnicodeBlock::YiSyllables | UnicodeBlock::YiRadicals => {
                        families.push("Microsoft Yi Baiti");
                        families.push("Segoe UI");
                    },

                    UnicodeBlock::Vai |
                    UnicodeBlock::CyrillicExtendedB |
                    UnicodeBlock::Bamum |
                    UnicodeBlock::ModifierToneLetters |
                    UnicodeBlock::LatinExtendedD => {
                        families.push("Ebrima");
                        families.push("Segoe UI");
                        families.push("Cambria Math");
                    },

                    UnicodeBlock::SylotiNagri |
                    UnicodeBlock::CommonIndicNumberForms |
                    UnicodeBlock::Phagspa |
                    UnicodeBlock::Saurashtra |
                    UnicodeBlock::DevanagariExtended => {
                        families.push("Microsoft PhagsPa");
                        families.push("Nirmala UI");
                    },

                    UnicodeBlock::KayahLi | UnicodeBlock::Rejang | UnicodeBlock::Javanese => {
                        families.push("Malgun Gothic");
                        families.push("Javanese Text");
                        families.push("Leelawadee UI");
                    },

                    UnicodeBlock::AlphabeticPresentationForms => {
                        families.push("Microsoft Uighur");
                        families.push("Gabriola");
                        families.push("Sylfaen");
                    },

                    UnicodeBlock::ArabicPresentationFormsA |
                    UnicodeBlock::ArabicPresentationFormsB => {
                        families.push("Traditional Arabic");
                        families.push("Arabic Typesetting");
                    },

                    UnicodeBlock::VariationSelectors |
                    UnicodeBlock::VerticalForms |
                    UnicodeBlock::CombiningHalfMarks |
                    UnicodeBlock::CJKCompatibilityForms |
                    UnicodeBlock::SmallFormVariants |
                    UnicodeBlock::HalfwidthandFullwidthForms |
                    UnicodeBlock::Specials => {
                        families.push("Microsoft JhengHei");
                    },

                    _ => {},
                }
            }
        },

        // https://en.wikipedia.org/wiki/Plane_(Unicode)#Supplementary_Multilingual_Plane
        1 => {
            families.push("Segoe UI Symbol");
            families.push("Ebrima");
            families.push("Nirmala UI");
            families.push("Cambria Math");
        },

        _ => {},
    }

    families.push("Arial Unicode MS");
    families
}

fn font_template_descriptor_from_font(font: &Font) -> FontTemplateDescriptor {
    let style = match font.style() {
        FontStyle::Normal => StyleFontStyle::NORMAL,
        FontStyle::Oblique => StyleFontStyle::OBLIQUE,
        FontStyle::Italic => StyleFontStyle::ITALIC,
    };
    let weight = StyleFontWeight::from_float(font.weight().to_u32() as f32);
    let width = match font.stretch() {
        FontStretch::Undefined => FontWidthKeyword::Normal,
        FontStretch::UltraCondensed => FontWidthKeyword::UltraCondensed,
        FontStretch::ExtraCondensed => FontWidthKeyword::ExtraCondensed,
        FontStretch::Condensed => FontWidthKeyword::Condensed,
        FontStretch::SemiCondensed => FontWidthKeyword::SemiCondensed,
        FontStretch::Normal => FontWidthKeyword::Normal,
        FontStretch::SemiExpanded => FontWidthKeyword::SemiExpanded,
        FontStretch::Expanded => FontWidthKeyword::Expanded,
        FontStretch::ExtraExpanded => FontWidthKeyword::ExtraExpanded,
        FontStretch::UltraExpanded => FontWidthKeyword::UltraExpanded,
    }
    .compute();
    FontTemplateDescriptor::new(weight, width, style)
}

pub(crate) fn default_system_generic_font_family(
    generic: GenericFontFamily,
) -> LowercaseFontFamilyName {
    match generic {
        GenericFontFamily::None | GenericFontFamily::Serif => "Times New Roman",
        GenericFontFamily::SansSerif => "Arial",
        GenericFontFamily::Monospace => "Courier New",
        GenericFontFamily::Cursive => "Comic Sans MS",
        GenericFontFamily::Fantasy => "Impact",
        GenericFontFamily::SystemUi => "Segoe UI",
    }
    .into()
}
