//! Compile-time checked, zero-dependency internationalisation (`i18n`) support.
//!
//! Translation strings live in `<workspace>/locales/*.json`, compiled into static lookup tables by
//! `build.rs`. Static UI lookups return `&'static str` with O(1) indexing and zero allocations.
//!
//! # Turkish casing rule
//!
//! Never call Rust's `.to_uppercase()` or `.to_lowercase()` on translated Turkish strings
//! (standard Unicode case folding maps `i` to `I` instead of `İ`, and `I` to `i` instead of `ı`).
//! Store exact casing in `locales/tr.json`.

include!(concat!(env!("OUT_DIR"), "/i18n_generated.rs"));

/// Supported display locales.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Locale {
    #[default]
    En,
    Tr,
}

/// Grammatical plural category used to select pluralized catalog keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PluralCategory {
    One,
    Other,
}

impl Locale {
    /// Every supported locale, in menu order.
    pub const ALL: &'static [Locale] = &[Locale::En, Locale::Tr];

    /// BCP-47 primary language subtag (`"en"`, `"tr"`).
    pub const fn code(self) -> &'static str {
        match self {
            Locale::En => "en",
            Locale::Tr => "tr",
        }
    }

    /// Native display name shown in language pickers (`"English"`, `"Türkçe"`).
    pub const fn native_name(self) -> &'static str {
        match self {
            Locale::En => "English",
            Locale::Tr => "Türkçe",
        }
    }

    /// Parse a BCP-47 or POSIX locale tag (e.g. `"tr"`, `"tr_TR.UTF-8"`, `"en-US"`).
    /// Returns `None` for unsupported languages and neutral tags such as `"C"` or `"POSIX"`.
    pub fn parse(tag: &str) -> Option<Self> {
        let trimmed = tag.trim();
        if trimmed.is_empty() {
            return None;
        }
        let no_encoding = trimmed.split(['.', '@']).next().unwrap_or("");
        let primary = no_encoding.split(['-', '_']).next().unwrap_or("");
        if primary.eq_ignore_ascii_case("en") {
            Some(Locale::En)
        } else if primary.eq_ignore_ascii_case("tr") {
            Some(Locale::Tr)
        } else {
            None
        }
    }

    /// Detect the user's preferred locale from the operating system, falling back to `En`.
    pub fn detect_system() -> Self {
        #[cfg(windows)]
        {
            if let Some(tag) = windows_locale_name() {
                if let Some(loc) = Self::parse(&tag) {
                    return loc;
                }
            }
            Locale::En
        }
        #[cfg(not(windows))]
        {
            for var in ["LC_ALL", "LC_MESSAGES", "LANG"] {
                if let Ok(val) = std::env::var(var) {
                    let trimmed = val.trim();
                    if trimmed.is_empty()
                        || trimmed.eq_ignore_ascii_case("C")
                        || trimmed.eq_ignore_ascii_case("POSIX")
                        || trimmed.starts_with("C.")
                    {
                        continue;
                    }
                    if let Some(loc) = Self::parse(trimmed) {
                        return loc;
                    }
                    // A valid non-empty LC_ALL/LC_MESSAGES naming an unsupported locale
                    // (e.g. de_DE) stops fallback to lower-precedence env vars.
                    return Locale::En;
                }
            }
            Locale::En
        }
    }

    /// Read `SEACRAFT_LANG`, if set and valid.
    pub fn from_env() -> Option<Self> {
        std::env::var("SEACRAFT_LANG")
            .ok()
            .and_then(|v| Self::parse(&v))
    }

    /// Plural category for `count` in this locale.
    pub const fn plural_category(self, count: usize) -> PluralCategory {
        match self {
            Locale::En | Locale::Tr => {
                if count == 1 {
                    PluralCategory::One
                } else {
                    PluralCategory::Other
                }
            }
        }
    }
}

#[cfg(windows)]
fn windows_locale_name() -> Option<String> {
    const LOCALE_NAME_MAX_LENGTH: usize = 85;

    #[link(name = "kernel32")]
    extern "system" {
        fn GetUserDefaultLocaleName(lp_locale_name: *mut u16, cch_locale_name: i32) -> i32;
    }

    let mut buf = [0u16; LOCALE_NAME_MAX_LENGTH];
    // SAFETY: `buf` is a writable UTF-16 buffer of `LOCALE_NAME_MAX_LENGTH` elements.
    let len = unsafe { GetUserDefaultLocaleName(buf.as_mut_ptr(), buf.len() as i32) };
    if len <= 0 {
        return None;
    }
    let slice = &buf[..(len as usize).min(buf.len())];
    let end = slice.iter().position(|&c| c == 0).unwrap_or(slice.len());
    String::from_utf16(&slice[..end]).ok()
}

/// Persisted user preference: follow the system locale or pin a specific language.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LanguagePref {
    #[default]
    Auto,
    Fixed(Locale),
}

impl LanguagePref {
    /// Wire/storage representation (`"auto"`, `"en"`, `"tr"`).
    pub const fn as_str(self) -> &'static str {
        match self {
            LanguagePref::Auto => "auto",
            LanguagePref::Fixed(loc) => loc.code(),
        }
    }

    /// Parse from storage (`"auto"` or a locale tag; unknown values fall back to `Auto`).
    pub fn parse(s: &str) -> Self {
        let trimmed = s.trim();
        if trimmed.eq_ignore_ascii_case("auto") {
            return LanguagePref::Auto;
        }
        match Locale::parse(trimmed) {
            Some(loc) => LanguagePref::Fixed(loc),
            None => LanguagePref::Auto,
        }
    }

    /// Effective locale given the detected `system` locale.
    pub const fn resolve(self, system: Locale) -> Locale {
        match self {
            LanguagePref::Auto => system,
            LanguagePref::Fixed(loc) => loc,
        }
    }
}

/// Resolve the active locale according to precedence:
/// `--lang` CLI flag > `SEACRAFT_LANG` env > saved preference > OS system locale.
pub fn resolve_locale(
    cli: Option<Locale>,
    env: Option<Locale>,
    pref: LanguagePref,
    system: Locale,
) -> Locale {
    cli.or(env).unwrap_or_else(|| pref.resolve(system))
}

/// Lightweight translator value passed into UI panels, CLI printers and document builders.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Tr(pub Locale);

impl Tr {
    pub const fn new(locale: Locale) -> Self {
        Self(locale)
    }

    pub const fn locale(self) -> Locale {
        self.0
    }

    /// Look up a static string by compile-time checked `Key`.
    pub const fn s(self, key: Key) -> &'static str {
        let idx = key as usize;
        match self.0 {
            Locale::En => STRINGS_EN[idx],
            Locale::Tr => STRINGS_TR[idx],
        }
    }

    /// Format `key` by replacing `{name}` placeholders with matching values in `args`.
    pub fn fmt(self, key: Key, args: &[(&str, &str)]) -> String {
        let template = self.s(key);
        let mut out = String::with_capacity(template.len() + 16);
        let mut rest = template;
        while let Some(open) = rest.find('{') {
            out.push_str(&rest[..open]);
            let after_open = &rest[open + 1..];
            if let Some(close) = after_open.find('}') {
                let name = &after_open[..close];
                if let Some((_, val)) = args.iter().find(|(k, _)| *k == name) {
                    out.push_str(val);
                } else {
                    out.push('{');
                    out.push_str(name);
                    out.push('}');
                }
                rest = &after_open[close + 1..];
            } else {
                out.push('{');
                rest = after_open;
            }
        }
        out.push_str(rest);
        out
    }

    /// Format a pluralized message using `{count}` and this locale's plural rule.
    pub fn plural(self, one: Key, other: Key, count: usize) -> String {
        let key = match self.0.plural_category(count) {
            PluralCategory::One => one,
            PluralCategory::Other => other,
        };
        let count_str = count.to_string();
        self.fmt(key, &[("count", &count_str)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locale_parse_handles_bcp47_and_posix_tags() {
        assert_eq!(Locale::parse("en"), Some(Locale::En));
        assert_eq!(Locale::parse("en-US"), Some(Locale::En));
        assert_eq!(Locale::parse("en_GB.UTF-8"), Some(Locale::En));
        assert_eq!(Locale::parse("tr"), Some(Locale::Tr));
        assert_eq!(Locale::parse("tr_TR"), Some(Locale::Tr));
        assert_eq!(Locale::parse("tr_TR.UTF-8"), Some(Locale::Tr));
        assert_eq!(Locale::parse("TR-tr@latin"), Some(Locale::Tr));
        assert_eq!(Locale::parse("C"), None);
        assert_eq!(Locale::parse("POSIX"), None);
        assert_eq!(Locale::parse(""), None);
        assert_eq!(Locale::parse("de"), None);
    }

    #[test]
    fn resolve_locale_respects_priority() {
        assert_eq!(
            resolve_locale(
                Some(Locale::En),
                Some(Locale::Tr),
                LanguagePref::Fixed(Locale::Tr),
                Locale::Tr
            ),
            Locale::En
        );
        assert_eq!(
            resolve_locale(
                None,
                Some(Locale::Tr),
                LanguagePref::Fixed(Locale::En),
                Locale::En
            ),
            Locale::Tr
        );
        assert_eq!(
            resolve_locale(None, None, LanguagePref::Fixed(Locale::Tr), Locale::En),
            Locale::Tr
        );
        assert_eq!(
            resolve_locale(None, None, LanguagePref::Auto, Locale::Tr),
            Locale::Tr
        );
    }

    #[test]
    fn translator_static_fmt_and_plural_work() {
        let en = Tr::new(Locale::En);
        let tr = Tr::new(Locale::Tr);

        assert_eq!(en.s(Key::MenuHelp), "Help");
        assert_eq!(tr.s(Key::MenuHelp), "Yardım");
        assert_eq!(
            en.fmt(Key::LangSystemDefault, &[("lang", "English")]),
            "System default (English)"
        );
        assert_eq!(
            tr.fmt(Key::LangSystemDefault, &[("lang", "Türkçe")]),
            "Sistem varsayılanı (Türkçe)"
        );
        assert_eq!(
            en.plural(Key::StatusErrorsOne, Key::StatusErrorsOther, 1),
            "1 error"
        );
        assert_eq!(
            en.plural(Key::StatusErrorsOne, Key::StatusErrorsOther, 3),
            "3 errors"
        );
        assert_eq!(
            tr.plural(Key::StatusErrorsOne, Key::StatusErrorsOther, 1),
            "1 hata"
        );
        assert_eq!(
            tr.plural(Key::StatusErrorsOne, Key::StatusErrorsOther, 3),
            "3 hata"
        );
    }

    #[test]
    fn all_catalog_characters_exist_in_embedded_dejavu_font() {
        let font = crate::ttf::Font::parse(crate::fonts::DEJAVU_SANS)
            .expect("embedded DejaVu Sans parses");
        for &loc in Locale::ALL {
            let tr = Tr::new(loc);
            for &key in Key::ALL {
                let text = tr.s(key);
                for ch in text.chars() {
                    if ch.is_control() {
                        continue;
                    }
                    assert!(
                        font.glyph(ch).is_some(),
                        "character '{ch}' (U+{:04X}) in locale '{}' key '{}' missing from DejaVuSans.ttf",
                        ch as u32,
                        loc.code(),
                        key.id()
                    );
                }
            }
        }
    }
}
