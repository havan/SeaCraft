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

    /// Parse a language code given by a user (`--lang`, MCP `locale`): exactly `"en"` or `"tr"`,
    /// ignoring ASCII case. Region and encoding suffixes are refused, unlike [`Locale::parse`],
    /// which reads operating-system tags.
    pub fn from_code(code: &str) -> Option<Self> {
        Self::ALL
            .iter()
            .copied()
            .find(|loc| code.trim().eq_ignore_ascii_case(loc.code()))
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
            Self::detect_from_env(|var| std::env::var(var).ok())
        }
    }

    /// The POSIX message locale from `LC_ALL`, `LC_MESSAGES` and `LANG`, read through `get`.
    ///
    /// The first set, non-empty variable decides, as POSIX says: `C`, `C.*` and `POSIX` mean
    /// English, and so does a language SeaCraft does not have. GNU `LANGUAGE` is not consulted.
    pub fn detect_from_env(get: impl Fn(&str) -> Option<String>) -> Self {
        for var in ["LC_ALL", "LC_MESSAGES", "LANG"] {
            let Some(val) = get(var) else { continue };
            let trimmed = val.trim();
            if trimmed.is_empty() {
                continue;
            }
            if trimmed == "C" || trimmed == "POSIX" || trimmed.starts_with("C.") {
                return Locale::En;
            }
            return Self::parse(trimmed).unwrap_or(Locale::En);
        }
        Locale::En
    }

    /// Read `SEACRAFT_LANG`, if set and valid. [`Locale::from_env_checked`] also reports a value
    /// that names no supported language, so callers can warn about it.
    pub fn from_env() -> Option<Self> {
        Self::from_env_checked().ok().flatten()
    }

    /// `SEACRAFT_LANG`: `Ok(None)` when unset or empty, `Err(value)` when it names no supported
    /// language. It takes a code (`tr`) or an operating-system tag (`tr_TR.UTF-8`).
    pub fn from_env_checked() -> Result<Option<Self>, String> {
        Self::check_env_value(std::env::var("SEACRAFT_LANG").ok())
    }

    fn check_env_value(value: Option<String>) -> Result<Option<Self>, String> {
        match value {
            None => Ok(None),
            Some(v) if v.trim().is_empty() => Ok(None),
            Some(v) => Self::parse(&v).map(Some).ok_or(v),
        }
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
    pub fn s(self, key: Key) -> &'static str {
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
                    debug_assert!(
                        false,
                        "i18n key '{}' ({}): no value for placeholder '{{{name}}}'",
                        key.id(),
                        self.0.code()
                    );
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

    /// Render a structured [`Msg`] in this locale.
    pub fn msg(self, msg: &Msg) -> String {
        msg.render(self)
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

/// A user-facing message kept as a catalog key and its already formatted arguments, so the
/// computation that raises it does not have to know the reader's language.
///
/// Core results (method-validity warnings, wave notes, loading and damage warnings) carry `Msg`
/// and every human-readable output renders it with its own [`Tr`]. `Display` and `Serialize`
/// render English, so `--json`, MCP replies and saved files stay invariant across locales.
/// A message read back from JSON has lost its key and is kept as [`Msg::Raw`] text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Msg {
    /// A catalog key and its `{placeholder}` values, each rendered in the same language.
    Keyed {
        key: Key,
        args: Vec<(&'static str, Msg)>,
    },
    /// Text that is shown as it is in every locale.
    Raw(String),
}

impl Msg {
    /// A message with no arguments yet.
    pub fn new(key: Key) -> Self {
        Msg::Keyed {
            key,
            args: Vec::new(),
        }
    }

    /// Add the value of `{name}`. Numbers are formatted by the caller, with `.` as the decimal
    /// separator in every locale.
    pub fn arg(self, name: &'static str, value: impl std::fmt::Display) -> Self {
        self.arg_msg(name, Msg::Raw(value.to_string()))
    }

    /// Add the value of `{name}` as a message of its own, rendered in the reader's language too.
    pub fn arg_msg(mut self, name: &'static str, value: Msg) -> Self {
        if let Msg::Keyed { args, .. } = &mut self {
            args.push((name, value));
        }
        self
    }

    /// The catalog key, when the message has one.
    pub fn key(&self) -> Option<Key> {
        match self {
            Msg::Keyed { key, .. } => Some(*key),
            Msg::Raw(_) => None,
        }
    }

    /// The message in `tr`'s language.
    pub fn render(&self, tr: Tr) -> String {
        match self {
            Msg::Keyed { key, args } => {
                let values: Vec<(&str, String)> =
                    args.iter().map(|(k, v)| (*k, v.render(tr))).collect();
                let args: Vec<(&str, &str)> =
                    values.iter().map(|(k, v)| (*k, v.as_str())).collect();
                tr.fmt(*key, &args)
            }
            Msg::Raw(text) => text.clone(),
        }
    }
}

impl From<String> for Msg {
    fn from(text: String) -> Self {
        Msg::Raw(text)
    }
}

impl std::fmt::Display for Msg {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.render(Tr::new(Locale::En)))
    }
}

impl serde::Serialize for Msg {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> serde::Deserialize<'de> for Msg {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer).map(Msg::Raw)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn msg_renders_per_locale_and_serializes_english() {
        let m = Msg::new(Key::LangSystemDefault).arg("lang", "X");
        assert_eq!(m.render(Tr::new(Locale::En)), "System default (X)");
        assert_eq!(m.render(Tr::new(Locale::Tr)), "Sistem varsayılanı (X)");
        // A message inside a message follows the reader's language too.
        let nested =
            Msg::new(Key::LangSystemDefault).arg_msg("lang", Msg::new(Key::TemplatesSheetProfile));
        assert_eq!(
            nested.render(Tr::new(Locale::Tr)),
            "Sistem varsayılanı (Profil)"
        );
        assert_eq!(nested.to_string(), "System default (Profile)");
        assert_eq!(m.to_string(), "System default (X)");
        let json = serde_json::to_string(&m).unwrap();
        assert_eq!(json, "\"System default (X)\"");
        let back: Msg = serde_json::from_str(&json).unwrap();
        assert_eq!(back, Msg::Raw("System default (X)".into()));
        assert_eq!(back.render(Tr::new(Locale::Tr)), "System default (X)");
    }

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
    fn from_code_accepts_only_supported_codes() {
        assert_eq!(Locale::from_code("en"), Some(Locale::En));
        assert_eq!(Locale::from_code("TR"), Some(Locale::Tr));
        assert_eq!(Locale::from_code(" tr "), Some(Locale::Tr));
        assert_eq!(Locale::from_code("tr_TR.UTF-8"), None);
        assert_eq!(Locale::from_code("EN-zz"), None);
        assert_eq!(Locale::from_code("tr-XX"), None);
        assert_eq!(Locale::from_code("de"), None);
        assert_eq!(Locale::from_code(""), None);
    }

    fn env(pairs: &'static [(&'static str, &'static str)]) -> impl Fn(&str) -> Option<String> {
        move |var| {
            pairs
                .iter()
                .find(|(k, _)| *k == var)
                .map(|(_, v)| (*v).to_string())
        }
    }

    #[test]
    fn detect_from_env_follows_posix_precedence() {
        let d = Locale::detect_from_env;
        assert_eq!(d(env(&[])), Locale::En);
        assert_eq!(d(env(&[("LANG", "tr_TR.UTF-8")])), Locale::Tr);
        // A set LC_ALL overrides everything, and C or POSIX there means English.
        assert_eq!(
            d(env(&[("LC_ALL", "C"), ("LANG", "tr_TR.UTF-8")])),
            Locale::En
        );
        assert_eq!(
            d(env(&[("LC_ALL", "POSIX"), ("LC_MESSAGES", "tr_TR.UTF-8")])),
            Locale::En
        );
        assert_eq!(
            d(env(&[("LC_ALL", "C.UTF-8"), ("LANG", "tr_TR.UTF-8")])),
            Locale::En
        );
        assert_eq!(
            d(env(&[("LC_MESSAGES", "C"), ("LANG", "tr_TR.UTF-8")])),
            Locale::En
        );
        // Empty variables are skipped.
        assert_eq!(
            d(env(&[("LC_ALL", ""), ("LANG", "tr_TR.UTF-8")])),
            Locale::Tr
        );
        assert_eq!(
            d(env(&[("LC_ALL", "tr_TR.UTF-8"), ("LANG", "en_US.UTF-8")])),
            Locale::Tr
        );
        // An unsupported language decides too: English, not the next variable.
        assert_eq!(
            d(env(&[
                ("LC_MESSAGES", "de_DE.UTF-8"),
                ("LANG", "tr_TR.UTF-8")
            ])),
            Locale::En
        );
    }

    #[test]
    fn seacraft_lang_value_is_checked() {
        assert_eq!(Locale::check_env_value(None), Ok(None));
        assert_eq!(Locale::check_env_value(Some(" ".into())), Ok(None));
        assert_eq!(
            Locale::check_env_value(Some("tr".into())),
            Ok(Some(Locale::Tr))
        );
        assert_eq!(
            Locale::check_env_value(Some("tr_TR.UTF-8".into())),
            Ok(Some(Locale::Tr))
        );
        assert_eq!(Locale::check_env_value(Some("de".into())), Err("de".into()));
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
