use crate::github::{self, pull_request, PullRequestOptions};
use git2::{PushOptions, RemoteCallbacks, Repository, Tree};
use glacier::TestResult;
use std::error::Error;
use std::path::{Path, PathBuf};

const FIXED_DIR: &str = "fixed";

struct Descriptions {
    commit_message: String,
    pr_title: String,
    pr_body: String,
}

fn format_descriptions(test: &TestResult) -> Option<Descriptions> {
    let title = test.title();
    let description = test.description()?;

    let commit_message = format!("{}\n\n{}\n", title, description);

    let issue = format!("Issue: {}", test.issue_url()?);
    let description_codeblock = format!("```\n{}\n```", description);
    let pr_body = format!("{}\n\n{}\n", issue, description_codeblock);

    Some(Descriptions {
        commit_message,
        pr_title: title,
        pr_body,
    })
}

fn new_path_bytes(old: &Path) -> Option<Vec<u8>> {
    let mut new = PathBuf::from(FIXED_DIR);

    new.push(old.file_name()?);

    Some(new.to_str()?.as_bytes().to_vec())
}

fn move_to_fixed<'a>(repo: &'a Repository, test: &TestResult) -> Result<Tree<'a>, Box<dyn Error>> {
    let mut index = repo.index()?;

    // Stage 0 = normal file, not part of a merge
    let mut entry = index
        .get_path(test.path(), 0)
        .ok_or_else(|| format!("not found in index: {}", test.path().display()))?;

    index.remove(test.path(), 0)?;

    let new_path =
        new_path_bytes(test.path()).ok_or_else(|| format!("ICE has no filename: {:?}", test))?;

    entry.path = new_path;

    index.add(&entry)?;

    let id = index.write_tree()?;

    Ok(repo.find_tree(id)?)
}

fn push(repo: &Repository, refspec: &str, config: &github::Config) -> Result<(), Box<dyn Error>> {
    let mut callbacks = RemoteCallbacks::new();

    callbacks.push_update_reference(|_ref, status| match status {
        None => Ok(()),
        Some(message) => Err(git2::Error::from_str(message)),
    });

    let mut opts = PushOptions::new();
    opts.remote_callbacks(callbacks);

    let remote_url = config.remote_url();

    let mut remote = repo.remote_anonymous(&remote_url)?;
    remote.push(&[refspec], Some(&mut opts))?;

    Ok(())
}

pub(crate) fn fix(test: &TestResult, config: &github::Config) -> Result<(), Box<dyn Error>> {
    let repo = Repository::open(".")?;

    let path = test.path().display();
    let local_branch = format!("refs/heads/autofix/{}", &path);
    let remote_branch = format!("refs/remotes/origin/autofix/{}", &path);

    if repo.find_reference(&remote_branch).is_ok() {
        println!("Branch exists: {}", remote_branch);
        return Ok(());
    }

    let descriptions = format_descriptions(test).ok_or("Failed to format Descriptions")?;

    let head = repo.head()?.peel_to_commit()?;
    let sig = repo.signature()?;
    let tree = move_to_fixed(&repo, &test)?;

    repo.commit(
        Some(&local_branch),
        &sig,
        &sig,
        &descriptions.commit_message,
        &tree,
        &[&head],
    )?;
    push(&repo, &local_branch, config)?;

    let url = pull_request(
        config,
        &PullRequestOptions {
            title: &descriptions.pr_title,
            body: &descriptions.pr_body,
            head: &local_branch,
            base: "master",
        },
    )?;

    println!("Created pull request: {}", url);
    Ok(())
}
