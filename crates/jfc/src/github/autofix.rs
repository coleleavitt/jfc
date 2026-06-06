//! `/pr-autofix <num>` — read PR review comments and produce a structured
//! prompt for the model.
//!
//! The actual "fix" is done by the model in the next turn — we don't run
//! `cargo fix` ourselves. Our job is to gather the review feedback and
//! frame it so the model can iterate on the user's working tree.
//!
//! Mirrors v2.1.132's `tengu_autofix_pr_*` flow: collect PR title/body, all
//! issue comments, and all line-level review comments, then format them as
//! a single prompt the LLM can act on.

use super::client::{GhClient, GhError, Pr, PrComment};

/// Build the autofix prompt from a fetched PR. The output is plain markdown
/// with `<system-reminder>`-style framing so the model treats it as a
/// directive rather than a chat turn.
pub fn build_autofix_prompt(pr: &Pr) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "PR #{num} (`{state}`) — {title}\n\nURL: {url}\n",
        num = pr.number,
        state = pr.state,
        title = pr.title,
        url = pr.url,
    ));
    if !pr.body.trim().is_empty() {
        out.push_str("\n## PR description\n\n");
        out.push_str(pr.body.trim());
        out.push('\n');
    }

    let issue_comments = &pr.comments;
    if !issue_comments.is_empty() {
        out.push_str("\n## Issue comments\n\n");
        for c in issue_comments {
            push_comment(&mut out, c);
        }
    }

    let review_count: usize = pr.reviews.iter().map(|r| r.comments.len()).sum();
    if review_count > 0 || pr.reviews.iter().any(|r| !r.body.trim().is_empty()) {
        out.push_str("\n## Review feedback\n\n");
        for review in &pr.reviews {
            if !review.body.trim().is_empty() {
                out.push_str(&format!(
                    "### {} (state: {})\n\n{}\n\n",
                    review.author.login,
                    if review.state.is_empty() {
                        "COMMENTED"
                    } else {
                        &review.state
                    },
                    review.body.trim()
                ));
            }
            for c in &review.comments {
                push_comment(&mut out, c);
            }
        }
    }

    if issue_comments.is_empty() && review_count == 0 {
        out.push_str("\n_(no review comments to address yet)_\n");
    }

    out.push_str(
        "\n---\n\n\
         **Task:** read the comments above and propose specific code changes that resolve each one. \
         For each comment:\n\
         1. Identify the file + line(s) it refers to (use `Grep` if the comment doesn't pin a path).\n\
         2. Explain the suggested fix in one sentence.\n\
         3. Apply it via `Edit` or `Write` so the next push picks it up.\n\
         If a comment is unclear or can't be addressed without more info, say so explicitly rather than guessing.\n",
    );
    out
}

fn push_comment(out: &mut String, c: &PrComment) {
    let when = if c.created_at.is_empty() {
        String::new()
    } else {
        format!(" — {}", c.created_at)
    };
    out.push_str(&format!(
        "- **@{author}**{when}:\n  > {body}\n\n",
        author = c.author.login,
        when = when,
        body = c.body.trim().replace('\n', "\n  > ")
    ));
}

/// Fetch the PR and build the autofix prompt in one shot.
pub async fn run(client: &GhClient, num: u64) -> Result<String, GhError> {
    let pr = client.gh_pr_view(num).await?;
    Ok(build_autofix_prompt(&pr))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::github::client::{Pr, PrAuthor, PrComment, PrReview};

    fn sample_pr() -> Pr {
        Pr {
            number: 42,
            title: "Add github integration".into(),
            body: "This adds /install-github-app and friends.".into(),
            state: "OPEN".into(),
            url: "https://github.com/owner/repo/pull/42".into(),
            comments: vec![PrComment {
                author: PrAuthor {
                    login: "alice".into(),
                },
                body: "Please rename `gh_api` to `request`.".into(),
                created_at: "2026-05-07T12:00:00Z".into(),
            }],
            reviews: vec![PrReview {
                author: PrAuthor {
                    login: "bob".into(),
                },
                state: "CHANGES_REQUESTED".into(),
                body: "Couple of suggestions inline.".into(),
                comments: vec![PrComment {
                    author: PrAuthor {
                        login: "bob".into(),
                    },
                    body: "Drop this `unwrap`, prefer `?` propagation.".into(),
                    created_at: "2026-05-07T13:00:00Z".into(),
                }],
            }],
            author: PrAuthor {
                login: "carol".into(),
            },
            head_ref_name: "feat/gh".into(),
            base_ref_name: "master".into(),
        }
    }

    #[test]
    fn build_autofix_prompt_includes_all_sections_normal() {
        let prompt = build_autofix_prompt(&sample_pr());
        assert!(prompt.contains("PR #42"));
        assert!(prompt.contains("Add github integration"));
        assert!(prompt.contains("PR description"));
        assert!(prompt.contains("Issue comments"));
        assert!(prompt.contains("Review feedback"));
        assert!(prompt.contains("@alice"));
        assert!(prompt.contains("@bob"));
        assert!(prompt.contains("Please rename `gh_api`"));
        assert!(prompt.contains("Drop this `unwrap`"));
        assert!(prompt.contains("Couple of suggestions inline."));
        assert!(prompt.contains("CHANGES_REQUESTED"));
        // The trailing instructions must reference the canonical tools.
        assert!(prompt.contains("Edit"));
        assert!(prompt.contains("Grep"));
    }

    #[test]
    fn build_autofix_prompt_with_no_comments_robust() {
        let mut pr = sample_pr();
        pr.comments.clear();
        pr.reviews.clear();
        let prompt = build_autofix_prompt(&pr);
        assert!(prompt.contains("no review comments"));
        // Still must contain task instructions even when there's nothing to address.
        assert!(prompt.contains("Task:"));
    }

    #[test]
    fn build_autofix_prompt_indents_multiline_quotes_normal() {
        let mut pr = sample_pr();
        pr.comments[0].body = "first line\nsecond line\nthird line".into();
        let prompt = build_autofix_prompt(&pr);
        // Each line of a multi-line comment must be quoted.
        assert!(prompt.contains("> first line"));
        assert!(prompt.contains("> second line"));
        assert!(prompt.contains("> third line"));
    }

    #[test]
    fn build_autofix_prompt_handles_empty_review_state_robust() {
        let mut pr = sample_pr();
        pr.reviews[0].state = String::new();
        let prompt = build_autofix_prompt(&pr);
        assert!(prompt.contains("state: COMMENTED"));
    }
}
