use crate::parse::text;
use crate::{GitError, Repo};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Remote {
    pub name: String,
    pub url: String,
    /// Only where it differs from `url`.
    pub push_url: Option<String>,
}

/// The repository's remotes in `git remote -v`'s order.
pub fn remotes(repo: &Repo) -> Result<Vec<Remote>, GitError> {
    repo.read(&["remote", "-v"], None, |out| Ok(parse_remotes(&text(out))))
}

fn parse_remotes(out: &str) -> Vec<Remote> {
    let mut all: Vec<Remote> = Vec::new();
    for line in out.lines() {
        let Some((name, rest)) = line.split_once('\t') else { continue };
        let (url, kind) = match rest.rsplit_once(' ') {
            Some((url, kind)) => (url, kind),
            None => (rest, "(fetch)"),
        };
        match all.iter_mut().find(|r| r.name == name) {
            Some(r) if kind == "(push)" && r.url != url => r.push_url = Some(url.to_string()),
            Some(_) => {}
            None => all.push(Remote { name: name.to_string(), url: url.to_string(), push_url: None }),
        }
    }
    all
}

/// `url` with its userinfo (a user, a token, a password) as `***`, as every URL is shown.
pub fn mask_userinfo(url: &str) -> String {
    let Some((scheme, rest)) = url.split_once("://") else { return url.to_string() };
    let authority_end = rest.find('/').unwrap_or(rest.len());
    match rest[..authority_end].rfind('@') {
        Some(at) => format!("{scheme}://***@{}", &rest[at + 1..]),
        None => url.to_string(),
    }
}

/// What a link on a hosting site shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Link<'a> {
    Branch(&'a str),
    Commit(&'a str),
}

/// The web page of `link` on the site `remote_url` lives on (GitHub, GitLab, Bitbucket, Azure DevOps); `None`
/// for any other host.
pub fn web_link(remote_url: &str, link: Link) -> Option<String> {
    let (host, path) = host_and_path(remote_url)?;
    let path = path.trim_matches('/').trim_end_matches(".git");
    let azure = |org: &str, project: &str, repo: &str| {
        let base = format!("https://dev.azure.com/{org}/{project}/_git/{repo}");
        Some(match link {
            Link::Branch(b) => format!("{base}?version=GB{b}"),
            Link::Commit(sha) => format!("{base}/commit/{sha}"),
        })
    };
    match host.as_str() {
        "github.com" => Some(format!("https://github.com/{path}/{}", match link {
            Link::Branch(b) => format!("tree/{b}"),
            Link::Commit(sha) => format!("commit/{sha}"),
        })),
        "gitlab.com" => Some(format!("https://gitlab.com/{path}/-/{}", match link {
            Link::Branch(b) => format!("tree/{b}"),
            Link::Commit(sha) => format!("commit/{sha}"),
        })),
        "bitbucket.org" => Some(format!("https://bitbucket.org/{path}/{}", match link {
            Link::Branch(b) => format!("branch/{b}"),
            Link::Commit(sha) => format!("commits/{sha}"),
        })),
        "dev.azure.com" => match path.split('/').collect::<Vec<_>>()[..] {
            [org, project, "_git", repo] => azure(org, project, repo),
            _ => None,
        },
        "ssh.dev.azure.com" => match path.split('/').collect::<Vec<_>>()[..] {
            ["v3", org, project, repo] => azure(org, project, repo),
            _ => None,
        },
        host => {
            let org = host.strip_suffix(".visualstudio.com")?;
            match path.split('/').collect::<Vec<_>>()[..] {
                [project, "_git", repo] => azure(org, project, repo),
                _ => None,
            }
        }
    }
}

/// A remote URL's host, lower case, and its path: `scheme://[user@]host[:port]/path` or scp's `[user@]host:path`.
fn host_and_path(url: &str) -> Option<(String, String)> {
    let (authority, path) = match url.split_once("://") {
        Some((_, rest)) => rest.split_once('/').map(|(a, p)| (a, p.to_string()))?,
        None => {
            let (authority, path) = url.split_once(':')?;
            if authority.contains('/') || authority.len() == 1 {
                return None;
            }
            (authority, path.to_string())
        }
    };
    let host = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    let host = host.split_once(':').map_or(host, |(h, _)| h);
    Some((host.to_ascii_lowercase(), path))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remotes_parse_with_a_push_url_only_when_it_differs() {
        let out = "origin\thttps://github.com/a/b.git (fetch)\norigin\thttps://github.com/a/b.git (push)\nfork\tC:/f.git (fetch)\nfork\tssh://h/f.git (push)\n";
        assert_eq!(parse_remotes(out), [
            Remote { name: "origin".into(), url: "https://github.com/a/b.git".into(), push_url: None },
            Remote { name: "fork".into(), url: "C:/f.git".into(), push_url: Some("ssh://h/f.git".into()) },
        ]);
    }

    #[test]
    fn userinfo_is_masked_and_nothing_else() {
        assert_eq!(mask_userinfo("https://user:ghp_secret@github.com/a/b.git"), "https://***@github.com/a/b.git");
        assert_eq!(mask_userinfo("https://token@host/x@y"), "https://***@host/x@y");
        assert_eq!(mask_userinfo("https://github.com/a/b.git"), "https://github.com/a/b.git");
        assert_eq!(mask_userinfo("git@github.com:a/b.git"), "git@github.com:a/b.git");
        assert_eq!(mask_userinfo("C:/repos/origin.git"), "C:/repos/origin.git");
    }

    #[test]
    fn links_are_built_for_the_known_hosts_only() {
        assert_eq!(web_link("git@github.com:a/b.git", Link::Branch("main")).as_deref(), Some("https://github.com/a/b/tree/main"));
        assert_eq!(web_link("https://u:p@github.com/a/b.git", Link::Commit("abc")).as_deref(), Some("https://github.com/a/b/commit/abc"));
        assert_eq!(web_link("ssh://git@gitlab.com/g/s/r.git", Link::Branch("x")).as_deref(), Some("https://gitlab.com/g/s/r/-/tree/x"));
        assert_eq!(web_link("https://bitbucket.org/t/r.git", Link::Commit("abc")).as_deref(), Some("https://bitbucket.org/t/r/commits/abc"));
        assert_eq!(web_link("https://org@dev.azure.com/org/p/_git/r", Link::Branch("main")).as_deref(), Some("https://dev.azure.com/org/p/_git/r?version=GBmain"));
        assert_eq!(web_link("git@ssh.dev.azure.com:v3/org/p/r", Link::Commit("abc")).as_deref(), Some("https://dev.azure.com/org/p/_git/r/commit/abc"));
        assert_eq!(web_link("https://org.visualstudio.com/p/_git/r", Link::Commit("abc")).as_deref(), Some("https://dev.azure.com/org/p/_git/r/commit/abc"));
        assert_eq!(web_link("C:/repos/origin.git", Link::Branch("main")), None);
        assert_eq!(web_link("https://git.example.com/a/b.git", Link::Branch("main")), None);
    }
}
