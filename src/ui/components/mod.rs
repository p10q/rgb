// UI components module
// This module will contain reusable UI components

pub mod file_tree;
pub mod git_status;
pub mod commit_dialog;

// Re-exports
pub use file_tree::FileTreeComponent;
pub use git_status::GitStatusComponent;
pub use commit_dialog::CommitDialog;