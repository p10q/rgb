use crate::workspace::TerminalId;
use anyhow::Result;
use git2::{
    BranchType, DiffOptions, Repository, Status, StatusOptions, Worktree as Git2Worktree,
};
use parking_lot::RwLock;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

pub struct GitManager {
    repo: Option<Repository>,
    worktrees: Arc<RwLock<HashMap<TerminalId, WorktreeInfo>>>,
    status_cache: Arc<RwLock<GitStatus>>,
    project_dir: PathBuf,
}

#[derive(Debug, Clone)]
pub struct WorktreeInfo {
    pub path: PathBuf,
    pub branch: String,
    pub terminal_id: TerminalId,
    pub last_sync: Instant,
    pub merge_status: MergeStatus,
}

#[derive(Debug, Clone)]
pub enum MergeStatus {
    Unmerged,
    Merged,
    Conflict { main_branch: String, worktree_branch: String },
}

#[derive(Debug, Clone, Default)]
pub struct GitStatus {
    pub modified_files: Vec<PathBuf>,
    pub staged_files: Vec<PathBuf>,
    pub untracked_files: Vec<PathBuf>,
    pub deleted_files: Vec<PathBuf>,
    pub conflicted_files: Vec<PathBuf>,
}

#[derive(Debug, Clone)]
pub struct DiffHunk {
    pub file: PathBuf,
    pub old_start: usize,
    pub old_lines: usize,
    pub new_start: usize,
    pub new_lines: usize,
    pub content: String,
}

impl GitManager {
    pub fn new(project_dir: &Path) -> Result<Self> {
        let repo = Repository::open(project_dir).ok();

        if let Some(ref r) = repo {
            tracing::info!("Git repository found at {:?}", r.path());
        } else {
            tracing::info!("No git repository found at {:?}", project_dir);
        }

        Ok(Self {
            repo,
            worktrees: Arc::new(RwLock::new(HashMap::new())),
            status_cache: Arc::new(RwLock::new(GitStatus::default())),
            project_dir: project_dir.to_path_buf(),
        })
    }

    pub fn is_git_repo(&self) -> bool {
        self.repo.is_some()
    }

    pub async fn create_worktree(&self, terminal_id: TerminalId) -> Result<PathBuf> {
        let repo = self.repo.as_ref()
            .ok_or_else(|| anyhow::anyhow!("Not a git repository"))?;

        // Generate unique branch name
        let branch_name = format!("terminal-{}", &terminal_id.to_string()[..8]);

        // Create worktree path in temp directory
        let worktree_dir = std::env::temp_dir()
            .join("rgb-worktrees")
            .join(terminal_id.to_string());

        // Ensure directory exists
        tokio::fs::create_dir_all(&worktree_dir).await?;

        // Get current branch
        let head = repo.head()?;
        let current_branch = head.shorthand().unwrap_or("main");

        // Create new branch from current
        let oid = head.target()
            .ok_or_else(|| anyhow::anyhow!("HEAD has no target"))?;
        let commit = repo.find_commit(oid)?;
        repo.branch(&branch_name, &commit, false)?;

        // Add worktree
        let worktree = repo.worktree(
            &branch_name,
            &worktree_dir,
            Some(&git2::WorktreeAddOptions::new()),
        )?;

        // Store worktree info
        let info = WorktreeInfo {
            path: worktree_dir.clone(),
            branch: branch_name,
            terminal_id,
            last_sync: Instant::now(),
            merge_status: MergeStatus::Unmerged,
        };

        self.worktrees.write().insert(terminal_id, info);

        Ok(worktree_dir)
    }

    pub async fn cleanup_worktree(&self, terminal_id: TerminalId) -> Result<()> {
        if let Some(info) = self.worktrees.write().remove(&terminal_id) {
            if let Some(repo) = &self.repo {
                // Prune worktree
                if let Ok(worktree) = repo.find_worktree(&info.branch) {
                    worktree.prune(Some(&mut git2::WorktreePruneOptions::new()))?;
                }

                // Delete branch
                if let Ok(mut branch) = repo.find_branch(&info.branch, BranchType::Local) {
                    branch.delete()?;
                }
            }

            // Remove directory
            if info.path.exists() {
                tokio::fs::remove_dir_all(&info.path).await?;
            }
        }

        Ok(())
    }

    pub async fn sync_worktree(&self, terminal_id: TerminalId) -> Result<()> {
        let info = self.worktrees.read().get(&terminal_id).cloned();

        if let Some(mut info) = info {
            if info.last_sync.elapsed().as_secs() < 300 {
                // Skip if synced recently
                return Ok(());
            }

            if let Some(repo) = &self.repo {
                // Open worktree repository
                let worktree_repo = Repository::open(&info.path)?;

                // Get current branch in main repo
                let main_head = repo.head()?;
                let main_branch = main_head.shorthand().unwrap_or("main");

                // Check for uncommitted changes
                let statuses = worktree_repo.statuses(Some(
                    StatusOptions::new()
                        .include_untracked(true)
                        .include_ignored(false),
                ))?;

                if !statuses.is_empty() {
                    // Has uncommitted changes, skip sync
                    return Ok(());
                }

                // Attempt to merge main branch
                let main_oid = main_head.target()
                    .ok_or_else(|| anyhow::anyhow!("Main HEAD has no target"))?;
                let main_commit = repo.find_commit(main_oid)?;

                let worktree_head = worktree_repo.head()?;
                let worktree_oid = worktree_head.target()
                    .ok_or_else(|| anyhow::anyhow!("Worktree HEAD has no target"))?;
                let worktree_commit = worktree_repo.find_commit(worktree_oid)?;

                // Check if merge is needed
                let merge_base = repo.merge_base(main_oid, worktree_oid)?;

                if merge_base != main_oid {
                    // Merge is needed
                    let mut merge_options = git2::MergeOptions::new();
                    let merge_analysis = worktree_repo.merge_analysis(&[&main_commit])?;

                    if merge_analysis.0.contains(git2::MergeAnalysis::FASTFORWARD) {
                        // Fast-forward merge
                        worktree_repo.checkout_tree(
                            main_commit.as_object(),
                            Some(&mut git2::build::CheckoutBuilder::new()),
                        )?;
                        worktree_repo.set_head_detached(main_oid)?;
                        info.merge_status = MergeStatus::Merged;
                    } else if merge_analysis.0.contains(git2::MergeAnalysis::NORMAL) {
                        // Regular merge needed
                        // TODO: Implement proper merge
                        info.merge_status = MergeStatus::Conflict {
                            main_branch: main_branch.to_string(),
                            worktree_branch: info.branch.clone(),
                        };
                    }
                }

                info.last_sync = Instant::now();
                self.worktrees.write().insert(terminal_id, info);
            }
        }

        Ok(())
    }

    pub async fn get_status(&self) -> Result<GitStatus> {
        if let Some(repo) = &self.repo {
            let mut status = GitStatus::default();

            let statuses = repo.statuses(Some(
                StatusOptions::new()
                    .include_untracked(true)
                    .include_ignored(false),
            ))?;

            for entry in statuses.iter() {
                let path = entry.path()
                    .map(PathBuf::from)
                    .unwrap_or_default();

                let flags = entry.status();

                if flags.contains(Status::WT_MODIFIED) {
                    status.modified_files.push(path.clone());
                }
                if flags.contains(Status::INDEX_NEW) || flags.contains(Status::INDEX_MODIFIED) {
                    status.staged_files.push(path.clone());
                }
                if flags.contains(Status::WT_NEW) {
                    status.untracked_files.push(path.clone());
                }
                if flags.contains(Status::WT_DELETED) {
                    status.deleted_files.push(path.clone());
                }
                if flags.contains(Status::CONFLICTED) {
                    status.conflicted_files.push(path.clone());
                }
            }

            *self.status_cache.write() = status.clone();
            Ok(status)
        } else {
            Ok(GitStatus::default())
        }
    }

    pub async fn get_diff(&self, terminal_id: Option<TerminalId>) -> Result<Vec<DiffHunk>> {
        let mut hunks = Vec::new();

        if let Some(repo) = &self.repo {
            let repo_to_use = if let Some(tid) = terminal_id {
                if let Some(info) = self.worktrees.read().get(&tid) {
                    Repository::open(&info.path)?
                } else {
                    repo.clone()
                }
            } else {
                repo.clone()
            };

            let head = repo_to_use.head()?.peel_to_tree()?;
            let mut diff_options = DiffOptions::new();
            let diff = repo_to_use.diff_tree_to_workdir(Some(&head), Some(&mut diff_options))?;

            diff.foreach(
                &mut |delta, _| {
                    tracing::debug!("Diff file: {:?}", delta.new_file().path());
                    true
                },
                None,
                Some(&mut |delta, hunk| {
                    if let Some(path) = delta.new_file().path() {
                        hunks.push(DiffHunk {
                            file: path.to_path_buf(),
                            old_start: hunk.old_start() as usize,
                            old_lines: hunk.old_lines() as usize,
                            new_start: hunk.new_start() as usize,
                            new_lines: hunk.new_lines() as usize,
                            content: String::from_utf8_lossy(hunk.header()).to_string(),
                        });
                    }
                    true
                }),
                None,
            )?;
        }

        Ok(hunks)
    }

    pub async fn commit(
        &self,
        message: &str,
        files: Vec<PathBuf>,
        terminal_id: Option<TerminalId>,
    ) -> Result<String> {
        let repo = self.repo.as_ref()
            .ok_or_else(|| anyhow::anyhow!("Not a git repository"))?;

        let repo_to_use = if let Some(tid) = terminal_id {
            if let Some(info) = self.worktrees.read().get(&tid) {
                Repository::open(&info.path)?
            } else {
                repo.clone()
            }
        } else {
            repo.clone()
        };

        // Add files to index
        let mut index = repo_to_use.index()?;
        for file in files {
            index.add_path(&file)?;
        }
        index.write()?;

        // Create commit
        let signature = repo_to_use.signature()?;
        let tree_id = index.write_tree()?;
        let tree = repo_to_use.find_tree(tree_id)?;
        let parent_commit = repo_to_use.head()?.peel_to_commit()?;

        let commit_id = repo_to_use.commit(
            Some("HEAD"),
            &signature,
            &signature,
            message,
            &tree,
            &[&parent_commit],
        )?;

        Ok(commit_id.to_string())
    }
}