//! Offline repository URL construction. No checkout, credentials or provider API.

fn component(value: &str) -> String {
    let mut out = String::new();
    for b in value.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn decode(value: &str) -> Result<String, String> {
    let bytes = value.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = bytes.get(i + 1..i + 3).ok_or("invalid URL escape")?;
            let hex = std::str::from_utf8(hex).map_err(|_| "invalid URL escape")?;
            out.push(u8::from_str_radix(hex, 16).map_err(|_| "invalid URL escape")?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).map_err(|_| "URL path must be UTF-8".into())
}

fn path(value: &str, encoded: bool) -> Result<String, String> {
    if value.is_empty()
        || value.starts_with('/')
        || value.contains('\\')
        || value.chars().any(char::is_control)
    {
        return Err("source path must be a nonempty repository-relative path".into());
    }
    value
        .split('/')
        .map(|p| {
            let p = if encoded { decode(p)? } else { p.to_owned() };
            if p.is_empty() || matches!(p.as_str(), "." | "..") || p.contains(['/', '\\']) {
                return Err("ambiguous repository path component".into());
            }
            Ok(component(&p))
        })
        .collect::<Result<Vec<_>, _>>()
        .map(|p| p.join("/"))
}

/// Accept only ASCII DNS names and optional numeric HTTPS ports. IPv6 literals
/// require an explicit DNS web endpoint rather than guessing bracket/port rules.
fn host(value: &str, port: bool) -> Result<String, String> {
    let (name, number) = if port {
        value
            .split_once(':')
            .map_or((value, None), |(h, p)| (h, Some(p)))
    } else {
        (value, None)
    };
    if name.is_empty()
        || name.starts_with('.')
        || name.ends_with('.')
        || !name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b".-".contains(&b))
        || name
            .split('.')
            .any(|p| p.is_empty() || p.starts_with('-') || p.ends_with('-'))
    {
        return Err("unsupported repository host; configure a clean HTTPS DNS web base".into());
    }
    if let Some(number) = number
        && number.parse::<u16>().ok().is_none_or(|n| n == 0)
    {
        return Err("invalid repository port".into());
    }
    Ok(match number {
        Some(p) => format!("{}:{p}", name.to_ascii_lowercase()),
        None => name.to_ascii_lowercase(),
    })
}

pub(super) fn web_base(value: &str) -> Result<String, String> {
    let value = value
        .strip_prefix("https://")
        .ok_or("repository web base must use https://")?;
    if value.contains(['?', '#', '@']) || value.chars().any(char::is_control) {
        return Err("repository web base must not contain credentials, query or fragment".into());
    }
    let (authority, relative) = value
        .split_once('/')
        .ok_or("repository web base needs a repository path")?;
    Ok(format!(
        "https://{}/{}",
        host(authority, true)?,
        path(relative.trim_end_matches('/'), true)?
    ))
}

/// SSH transport ports and users are never copied into browser links. Unknown
/// hosts remain provider-ambiguous; callers must require an explicit mapping.
pub(super) fn repository(value: &str) -> Result<(String, Option<&'static str>), String> {
    let base = if value.starts_with("https://") {
        web_base(value)?
    } else {
        let ssh = value.strip_prefix("ssh://");
        let value = ssh.unwrap_or(value);
        if ssh.is_none()
            && (!value.contains('@') || value.split_once('@').is_some_and(|(u, _)| u.contains(':')))
        {
            return Err("repository needs an HTTPS or SSH remote".into());
        }
        let (authority, relative) = if ssh.is_some() {
            value.split_once('/').ok_or("SSH repository needs a path")?
        } else {
            value
                .split_once(':')
                .ok_or("repository needs an HTTPS or SSH remote")?
        };
        let authority = if let Some((user, h)) = authority.split_once('@') {
            if user.is_empty()
                || !user
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
            {
                return Err("SSH credentials are unsupported".into());
            }
            h
        } else {
            authority
        };
        let (name, port) = authority
            .split_once(':')
            .map_or((authority, None), |(h, p)| (h, Some(p)));
        if port.is_some_and(|p| p.parse::<u16>().ok().is_none_or(|n| n == 0)) {
            return Err("invalid SSH port".into());
        }
        format!(
            "https://{}/{}",
            host(name, false)?,
            path(relative, ssh.is_some())?
        )
    };
    let base = base.strip_suffix(".git").unwrap_or(&base).to_owned();
    let authority = base
        .strip_prefix("https://")
        .unwrap()
        .split('/')
        .next()
        .unwrap();
    let provider = match authority.split(':').next().unwrap() {
        "github.com" => Some("github"),
        "gitlab.com" => Some("gitlab"),
        _ => None,
    };
    Ok((base, provider))
}

pub(super) fn link(
    provider: &str,
    base: &str,
    revision: &str,
    file: &str,
    start: u64,
    end: u64,
) -> Result<String, String> {
    if !matches!(revision.len(), 40 | 64) || !revision.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("remote source requires a full immutable Git commit; dirty and symbolic revisions cannot be linked; remote commit availability is not checked".into());
    }
    if start == 0 || end < start {
        return Err("remote source requires an exact positive line range".into());
    }
    let base = web_base(base)?;
    let file = path(file, false)?;
    match provider {
        "github" => Ok(format!("{base}/blob/{revision}/{file}#L{start}-L{end}")),
        "gitlab" => Ok(format!("{base}/-/blob/{revision}/{file}#L{start}-{end}")),
        _ => Err("unsupported source provider; use github or gitlab".into()),
    }
}

/// Credential-free identity for matching a retained clone origin. SSH ports
/// belong to this identity, not to the HTTPS browser endpoint.
pub(super) fn remote_identity(value: &str) -> Result<String, String> {
    let (base, _) = repository(value)?;
    if value.starts_with("https://") {
        return Ok(base);
    }
    let authority = if let Some(rest) = value.strip_prefix("ssh://") {
        rest.split('/').next().unwrap()
    } else {
        value.split(':').next().unwrap()
    };
    let authority = authority.rsplit('@').next().unwrap();
    let authority = host(authority, true)?;
    let authority = authority.strip_suffix(":22").unwrap_or(&authority);
    let path = base
        .strip_prefix("https://")
        .unwrap()
        .split_once('/')
        .unwrap()
        .1;
    Ok(format!("ssh://{authority}/{path}"))
}

pub(super) fn file_matches(tail: &str, file: &str) -> bool {
    if tail == file
        || tail
            .strip_prefix(file)
            .is_some_and(|rest| rest.starts_with('#'))
    {
        return true;
    }
    tail.split('#')
        .next()
        .unwrap_or("")
        .split('/')
        .map(decode)
        .collect::<Result<Vec<_>, _>>()
        .is_ok_and(|parts| parts.join("/") == file)
}

#[cfg(test)]
mod tests {
    use super::*;
    const REV: &str = "0123456789abcdef0123456789abcdef01234567";
    #[test]
    fn capture_policy_separates_clone_origin_and_browser_base() {
        use super::super::{
            analysis::source_link,
            model::{Service, SourceWeb},
        };
        let mut service: Service = serde_json::from_value(serde_json::json!({
            "schema":"codeclew-documentation-service/1.0", "id":"weather", "title":"Weather",
            "repositoryId":"owned", "repository":"https://github.com/team/weather.git",
            "language":"java", "profile":"source-syntax", "targetRef":"main"
        }))
        .unwrap();
        let file = "src/space café/Weather.java";
        let expected = format!(
            "https://github.com/team/weather/blob/{REV}/src/space%20caf%C3%A9/Weather.java#L3-L5"
        );
        assert_eq!(source_link(&service, REV, file, 3, 5).unwrap(), expected);
        service.repository = ["git", "@", "github.com:team/weather.git"].concat();
        assert_eq!(source_link(&service, REV, file, 3, 5).unwrap(), expected);
        service.repository = "ssh://git@git.example.invalid:2222/team/weather.git".into();
        assert!(source_link(&service, REV, file, 3, 5).is_none());
        service.source_web = Some(SourceWeb {
            provider: "gitlab".into(),
            web_base: "https://git.example.invalid:8443/team/weather.git".into(),
        });
        assert_eq!(
            source_link(&service, REV, file, 3, 5).unwrap(),
            format!(
                "https://git.example.invalid:8443/team/weather.git/-/blob/{REV}/src/space%20caf%C3%A9/Weather.java#L3-5"
            )
        );
        service.source_link_template =
            Some("https://legacy.example.invalid/custom/{revision}/{file}".into());
        assert_eq!(
            source_link(&service, REV, file, 3, 5).unwrap(),
            format!(
                "https://legacy.example.invalid/custom/{REV}/src/space%20caf%C3%A9/Weather.java#L3-L5"
            )
        );
        service.source_web = None;
        assert!(
            serde_json::to_value(service)
                .unwrap()
                .get("sourceWeb")
                .is_none()
        );
        assert_eq!(
            repository(&["ssh://git", "@", "github.com:2222/team/A%20repo.git"].concat())
                .unwrap()
                .0,
            "https://github.com/team/A%20repo"
        );
    }

    #[test]
    fn github_special_file_components_and_ranges_are_exact() {
        let (base, provider) =
            repository(&["git", "@", "github.com:org/repo.git"].concat()).unwrap();
        assert_eq!(
            link(provider.unwrap(), &base, REV, "src/A file#?.kt", 7, 12).unwrap(),
            format!("https://github.com/org/repo/blob/{REV}/src/A%20file%23%3F.kt#L7-L12")
        );
        assert_eq!(
            repository("https://github.com/org/A%20repo.git").unwrap().0,
            "https://github.com/org/A%20repo"
        );
    }
    #[test]
    fn gitlab_nested_groups_unicode_and_transport_port_are_separate() {
        let (base, provider) =
            repository("ssh://git@git.example.invalid:2222/team/sub/repo.git").unwrap();
        assert_eq!(base, "https://git.example.invalid/team/sub/repo");
        assert_eq!(provider, None);
        let mapped = "https://git.example.invalid:8443/gitlab/team/sub/repo";
        assert_eq!(
            link(
                "gitlab",
                mapped,
                REV,
                "src/\u{41f}\u{440}\u{438}\u{43c}\u{435}\u{440}.cs",
                3,
                9
            )
            .unwrap(),
            format!("{mapped}/-/blob/{REV}/src/%D0%9F%D1%80%D0%B8%D0%BC%D0%B5%D1%80.cs#L3-9")
        );
    }
    #[test]
    fn refuses_credentials_unsafe_urls_and_ambiguous_paths() {
        for remote in [
            ["https://", "token", "@", "github.com/org/repo"].concat(),
            [
                "https://",
                "user",
                ":",
                "secret",
                "@",
                "github.com/org/repo",
            ]
            .concat(),
        ] {
            assert!(repository(&remote).is_err(), "{remote}");
        }
        for remote in [
            "javascript:alert(1)",
            "file:///repo",
            "https://github.com/org/repo?token=secret",
            "https://github.com/org/%2e%2e/repo",
        ] {
            assert!(repository(remote).is_err(), "{remote}");
        }
        for file in ["/src/A.kt", "../A.kt", "src/../A.kt", "src\\A.kt", ""] {
            assert!(link("github", "https://github.com/org/repo", REV, file, 1, 2).is_err());
        }
        for rev in [
            "HEAD",
            "main",
            "local:abc",
            "0123456789abcdef0123456789abcdef01234567-dirty",
            "",
        ] {
            assert!(link("github", "https://github.com/org/repo", rev, "A.kt", 1, 2).is_err());
        }
    }
    #[test]
    fn literal_percent_in_file_name_is_not_treated_as_a_url_escape() {
        assert!(
            link(
                "github",
                "https://github.com/org/repo",
                REV,
                "src/%2F.txt",
                1,
                1
            )
            .unwrap()
            .contains("/src/%252F.txt#L1-L1")
        );
        assert!(link("github", "https://github.com/org/repo", REV, "A.kt", 0, 2).is_err());
        assert!(link("gitlab", "https://gitlab.com/org/repo", REV, "A.kt", 3, 2).is_err());
    }
}
