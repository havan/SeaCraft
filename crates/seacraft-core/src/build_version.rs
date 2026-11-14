//! How a build names its version, from the package version and what git says about the source.
//! `build.rs` includes this file to stamp the build; the library compiles it only for its tests.
//!
//! | Build                                        | Version                    |
//! |----------------------------------------------|----------------------------|
//! | clean checkout of the tag `v0.3.0`           | `0.3.0`                    |
//! | 67 commits after `v0.3.0`                    | `0.3.0+67-ga4b51c2`        |
//! | the same with uncommitted changes            | `0.3.0+67-ga4b51c2-dirty`  |
//! | `Cargo.toml` says 0.3.1, no `v0.3.1` before  | `0.3.1-dev+gabc1234`       |
//! | a clone without tags, such as a shallow one  | `0.3.0+ga4b51c2`           |
//! | not a git checkout                           | `0.3.0+unknown`            |
//!
//! Only a clean release says the package version alone. `-dev` marks work towards a version not yet
//! tagged, so it orders before that release; what follows `+` names the commit and is ignored when
//! versions are ordered.

/// What git says about the checkout a build comes from.
pub struct Git {
    /// Commits since the tag `v<package version>`, or `None` when that tag is not an ancestor.
    pub since_release: Option<u32>,
    /// Whether the clone has any version tags; a shallow clone often has none.
    pub has_tags: bool,
    /// The abbreviated commit hash.
    pub hash: String,
    /// Whether tracked files differ from the commit.
    pub dirty: bool,
}

/// The version a build reports. `git` is `None` when the source is not a git checkout.
pub fn build_version(package: &str, git: Option<&Git>) -> String {
    let Some(git) = git else {
        return format!("{package}+unknown");
    };
    let dirty = if git.dirty { "-dirty" } else { "" };
    match git.since_release {
        Some(0) if !git.dirty => package.to_owned(),
        Some(n) => format!("{package}+{n}-g{}{dirty}", git.hash),
        None if git.has_tags => {
            // A pre-release version such as 0.3.0-beta.2 takes `dev` as one more identifier.
            let sep = if package.contains('-') { '.' } else { '-' };
            format!("{package}{sep}dev+g{}{dirty}", git.hash)
        }
        None => format!("{package}+g{}{dirty}", git.hash),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn git(since_release: Option<u32>, has_tags: bool, dirty: bool) -> Git {
        Git {
            since_release,
            has_tags,
            hash: "a4b51c2".into(),
            dirty,
        }
    }

    #[test]
    fn only_a_clean_release_says_the_package_version_alone() {
        assert_eq!(
            build_version("0.3.0", Some(&git(Some(0), true, false))),
            "0.3.0"
        );
        assert_eq!(
            build_version("0.3.0", Some(&git(Some(0), true, true))),
            "0.3.0+0-ga4b51c2-dirty"
        );
    }

    #[test]
    fn a_build_after_the_release_counts_the_commits() {
        assert_eq!(
            build_version("0.3.0", Some(&git(Some(67), true, false))),
            "0.3.0+67-ga4b51c2"
        );
        assert_eq!(
            build_version("0.3.0", Some(&git(Some(67), true, true))),
            "0.3.0+67-ga4b51c2-dirty"
        );
    }

    #[test]
    fn work_towards_an_untagged_version_is_a_dev_pre_release() {
        assert_eq!(
            build_version("0.3.1", Some(&git(None, true, false))),
            "0.3.1-dev+ga4b51c2"
        );
        assert_eq!(
            build_version("0.3.0-beta.2", Some(&git(None, true, true))),
            "0.3.0-beta.2.dev+ga4b51c2-dirty"
        );
    }

    #[test]
    fn without_tags_or_git_the_build_claims_nothing_about_a_release() {
        assert_eq!(
            build_version("0.3.0", Some(&git(None, false, false))),
            "0.3.0+ga4b51c2"
        );
        assert_eq!(build_version("0.3.0", None), "0.3.0+unknown");
    }
}
