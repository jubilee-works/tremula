//! Putting one comment on a pull request, and putting it there once.
//!
//! # Why it replaces rather than adds
//!
//! A pull request is pushed to many times, and every push produces evidence that supersedes
//! the last. A comment per push buries the review under stale results, each of them bound to
//! a commit nobody is looking at any more. So the comment carries a marker of its own on its
//! first line, and a run that finds that marker edits what is there instead of adding to it.
//!
//! The marker names the repository and the pull request, so two projects commenting on two
//! pull requests cannot mistake each other's comment for their own — and it is an HTML
//! comment, so nobody reading the thread ever sees it.
//!
//! # Why only a program's own comment is ever edited
//!
//! A token that may comment on a pull request may also edit anybody else's comment on it.
//! So the marker is not enough on its own: a person who quoted the marker, or who wrote it
//! out to ask what it was, would be holding a comment this could address a write to — and
//! that write would replace what they said with a mutation report. A comment is therefore
//! only ever replaced when the platform says a program wrote it *and* its first line is the
//! marker exactly. Anything else means there is nothing of ours on the pull request yet, and
//! a new comment is posted.
//!
//! # What is behind the boundary
//!
//! The choosing is a pure function over what the listing said, tested without a socket. What
//! is left needs a network: reading the list of comments, and one write. The endpoint is
//! injectable for the same reason the model provider's is — a test can answer as the real one
//! would, and no test needs the real one.
//!
//! The token is read from the environment at the moment of the call and never travels into a
//! message: what the API says is quoted back with anything key-shaped taken out, exactly as
//! the model provider's own answers are.

use std::time::Duration;

use serde_json::{Value, json};

use crate::generate::openai::{quote, redact};

/// Where the API is, unless a caller points this elsewhere.
pub const API: &str = "https://api.github.com";

/// Where the token is read from, every time a call is made.
pub const TOKEN_VARIABLE: &str = "GITHUB_TOKEN";

/// Where the repository is read from when the caller does not name one. What the platform's
/// own workflow runner sets, so a caller inside one names nothing.
pub const REPOSITORY_VARIABLE: &str = "GITHUB_REPOSITORY";

/// How long one call may take.
const TIMEOUT: Duration = Duration::from_secs(30);

/// How many comments to ask for at once, which is the most the API allows.
const PER_PAGE: usize = 100;

/// How many pages to read before giving up on finding the end of the list. A pull request
/// with more comments than this has other problems, and an unbounded loop against a paging
/// API is one nobody can interrupt.
const PAGES: usize = 20;

/// What this tool calls itself to a platform that asks.
const AGENT: &str = "tremula";

/// The marker that says a comment is this tool's, for one pull request of one repository.
#[must_use]
pub fn marker(repo: &str, pull_request: u64) -> String {
    format!("<!-- tremula-report:{repo}:{pull_request} -->")
}

/// The body as it is posted: the marker first, then the comment.
#[must_use]
pub fn with_marker(marker: &str, body: &str) -> String {
    format!("{marker}\n{body}")
}

/// One comment as the listing describes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listed {
    /// The identifier a write is addressed to.
    pub id: u64,
    /// Its body's first line, which is where a marker would be.
    pub first_line: String,
    /// Whether the platform says a program wrote it rather than a person.
    pub by_a_program: bool,
}

/// Which comment a new one replaces, if any of them is this tool's own.
///
/// Two conditions, and both of them are load-bearing.
///
/// The first line has to be the marker exactly. Not "contains": a comment that quoted a
/// previous one — a review, a bug report — would contain it, and editing somebody's account
/// of a problem is not something a reporting tool may do. Not "starts with", and not after a
/// trim either: a first line of the marker followed by a space is a line somebody typed, and
/// widening the comparison to accept it would widen it to accept them.
///
/// And the platform has to say a program wrote it. A token that may comment on a pull
/// request may also edit anybody else's comment on it, so a match on the marker alone is an
/// address a write could be sent to — and a person who wrote the marker out, for whatever
/// reason, would have their comment overwritten by a report. When nothing a program wrote
/// matches, there is nothing of ours here yet and a new comment is the answer.
///
/// Among the ones that do match, the newest is replaced, because the list is oldest first
/// and the newest is the one a reader is looking at.
#[must_use]
pub fn which_to_replace<'a>(listed: &'a [Listed], marker: &str) -> Option<&'a Listed> {
    listed
        .iter()
        .rev()
        .find(|comment| comment.by_a_program && comment.first_line == marker)
}

/// Where a comment ended up.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Posted {
    /// Where a reader can see it.
    pub url: String,
    /// Whether it replaced one that was already there.
    pub replaced: bool,
}

/// Why a comment could not be posted.
#[derive(Debug, thiserror::Error)]
pub enum PostError {
    /// No token in the environment.
    #[error(
        "no platform token in the environment; set `{variable}` to a token that may comment on this pull request — a workflow gives one out as `secrets.GITHUB_TOKEN` and needs `pull-requests: write` for it"
    )]
    MissingToken {
        /// The environment variable that was looked for.
        variable: String,
    },
    /// Nothing said which repository.
    #[error(
        "nothing says which repository the pull request is in; pass `--github-repo OWNER/NAME`, or run where `{variable}` is set, which every workflow of the platform does"
    )]
    NoRepository {
        /// The environment variable that would have said.
        variable: String,
    },
    /// The client itself could not be built.
    #[error("cannot make a call to the platform at all: {reason}")]
    NoClient {
        /// What the transport reported.
        reason: String,
    },
    /// The call never got an answer.
    #[error("could not reach the platform: {reason}; check the network and any proxy")]
    Unreachable {
        /// What the transport reported.
        reason: String,
    },
    /// The platform refused.
    #[error(
        "the platform refused the call (HTTP {status}): {detail}; check that the token may comment on this pull request — a workflow token needs `pull-requests: write` — and that the repository and the number are the right ones"
    )]
    Refused {
        /// The status it answered with.
        status: u16,
        /// What it said, with anything key-shaped taken out.
        detail: String,
    },
    /// What came back was not what this reads.
    #[error("what the platform answered is not the shape this reads: {reason}")]
    Unreadable {
        /// Why the answer could not be read.
        reason: String,
    },
}

/// Somewhere a comment can be put.
///
/// A trait rather than a function, so that everything above it can be exercised — the
/// choosing, the exit code a failed post produces, the body that was going to be sent —
/// without a network and without a token.
pub trait Poster {
    /// Put `body` on the pull request, replacing this tool's own comment if it is there.
    ///
    /// # Errors
    ///
    /// Returns [`PostError`] when the platform cannot be reached, refuses, or answers with
    /// something this cannot read.
    fn upsert(&self, repo: &str, pull_request: u64, body: &str) -> Result<Posted, PostError>;
}

/// The platform itself.
#[derive(Debug)]
pub struct GitHub {
    api: String,
    timeout: Duration,
}

impl Default for GitHub {
    fn default() -> Self {
        Self {
            api: API.to_owned(),
            timeout: TIMEOUT,
        }
    }
}

impl GitHub {
    /// The real platform.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The same, somewhere else. The seam a test answers through.
    #[must_use]
    pub fn with_api(mut self, api: String) -> Self {
        self.api = api;
        self
    }

    /// The same, with a limit of its own on how long one call may take.
    #[must_use]
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Every comment already on the pull request, however many pages that takes.
    fn listing(
        &self,
        client: &reqwest::blocking::Client,
        token: &str,
        repo: &str,
        pull_request: u64,
    ) -> Result<Vec<Listed>, PostError> {
        let mut found = Vec::new();
        for page in 1..=PAGES {
            let asked = format!(
                "{}/repos/{repo}/issues/{pull_request}/comments?per_page={PER_PAGE}&page={page}",
                self.api
            );
            let answered = Self::call(client, token, reqwest::Method::GET, &asked, None)?;
            let listed: Vec<Value> =
                serde_json::from_str(&answered).map_err(|err| PostError::Unreadable {
                    reason: err.to_string(),
                })?;
            let on_this_page = listed.len();
            found.extend(listed.iter().map(described));
            if on_this_page < PER_PAGE {
                break;
            }
        }
        Ok(found)
    }

    /// One call, with the token on a header and never on the payload.
    fn call(
        client: &reqwest::blocking::Client,
        token: &str,
        method: reqwest::Method,
        url: &str,
        payload: Option<&Value>,
    ) -> Result<String, PostError> {
        let mut request = client
            .request(method, url)
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .header("User-Agent", AGENT)
            .header("Authorization", format!("Bearer {token}"));
        if let Some(payload) = payload {
            request = request
                .header("Content-Type", "application/json")
                .body(payload.to_string());
        }
        let answered = request.send().map_err(|err| PostError::Unreachable {
            reason: redact(&err.to_string(), token),
        })?;
        let status = answered.status().as_u16();
        let body = answered.text().map_err(|err| PostError::Unreachable {
            reason: redact(&err.to_string(), token),
        })?;
        if !(200..300).contains(&status) {
            return Err(PostError::Refused {
                status,
                detail: quote(&body, token),
            });
        }
        Ok(body)
    }
}

impl Poster for GitHub {
    fn upsert(&self, repo: &str, pull_request: u64, body: &str) -> Result<Posted, PostError> {
        let Ok(token) = std::env::var(TOKEN_VARIABLE) else {
            return Err(PostError::MissingToken {
                variable: TOKEN_VARIABLE.to_owned(),
            });
        };
        let client = reqwest::blocking::Client::builder()
            .timeout(self.timeout)
            .build()
            .map_err(|err| PostError::NoClient {
                reason: redact(&err.to_string(), &token),
            })?;
        let listed = self.listing(&client, &token, repo, pull_request)?;
        let payload = json!({ "body": body });
        let marker = marker(repo, pull_request);
        let (method, url, replaced) = match which_to_replace(&listed, &marker) {
            Some(already) => (
                reqwest::Method::PATCH,
                format!("{}/repos/{repo}/issues/comments/{}", self.api, already.id),
                true,
            ),
            None => (
                reqwest::Method::POST,
                format!("{}/repos/{repo}/issues/{pull_request}/comments", self.api),
                false,
            ),
        };
        let answered = Self::call(&client, &token, method, &url, Some(&payload))?;
        let posted: Value =
            serde_json::from_str(&answered).map_err(|err| PostError::Unreadable {
                reason: err.to_string(),
            })?;
        Ok(Posted {
            url: posted["html_url"].as_str().unwrap_or_default().to_owned(),
            replaced,
        })
    }
}

/// One comment of the listing, as much of it as the choosing needs.
fn described(comment: &Value) -> Listed {
    let login = comment["user"]["login"].as_str().unwrap_or_default();
    Listed {
        id: comment["id"].as_u64().unwrap_or_default(),
        first_line: first_line(comment["body"].as_str().unwrap_or_default()),
        by_a_program: comment["user"]["type"].as_str() == Some("Bot") || login.ends_with("[bot]"),
    }
}

/// A body's first line, as the marker comparison sees it.
///
/// One carriage return comes off the end and nothing else does. A body stored with Windows
/// line endings would otherwise never match a marker this wrote, which is a difference in
/// how the text was transmitted rather than in what it says. Trailing space is a different
/// matter: it is something a person typed, and taking it off would make "the marker and a
/// space" match the marker — widening what a write may be addressed to, which is the one
/// thing the comparison is here to keep narrow.
#[must_use]
pub fn first_line(body: &str) -> String {
    let line = body.lines().next().unwrap_or_default();
    line.strip_suffix('\r').unwrap_or(line).to_owned()
}

/// Which repository a comment is about: the one the caller named, or the one the platform's
/// own workflow put in the environment.
///
/// # Errors
///
/// Returns [`PostError::NoRepository`] when neither says.
pub fn repository(named: Option<&str>) -> Result<String, PostError> {
    if let Some(named) = named {
        return Ok(named.to_owned());
    }
    std::env::var(REPOSITORY_VARIABLE).map_err(|_| PostError::NoRepository {
        variable: REPOSITORY_VARIABLE.to_owned(),
    })
}
