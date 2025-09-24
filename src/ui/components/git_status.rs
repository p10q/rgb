use crate::git::GitStatus;

pub struct GitStatusComponent {
    status: GitStatus,
    show_staged: bool,
    show_modified: bool,
    show_untracked: bool,
}

impl GitStatusComponent {
    pub fn new() -> Self {
        Self {
            status: GitStatus::default(),
            show_staged: true,
            show_modified: true,
            show_untracked: true,
        }
    }

    pub fn update_status(&mut self, status: GitStatus) {
        self.status = status;
    }

    pub fn toggle_staged(&mut self) {
        self.show_staged = !self.show_staged;
    }

    pub fn toggle_modified(&mut self) {
        self.show_modified = !self.show_modified;
    }

    pub fn toggle_untracked(&mut self) {
        self.show_untracked = !self.show_untracked;
    }
}