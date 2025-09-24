use std::path::PathBuf;

pub struct CommitDialog {
    message: String,
    files_to_commit: Vec<PathBuf>,
    is_open: bool,
}

impl CommitDialog {
    pub fn new() -> Self {
        Self {
            message: String::new(),
            files_to_commit: Vec::new(),
            is_open: false,
        }
    }

    pub fn open(&mut self, files: Vec<PathBuf>) {
        self.files_to_commit = files;
        self.is_open = true;
        self.message.clear();
    }

    pub fn close(&mut self) {
        self.is_open = false;
        self.message.clear();
        self.files_to_commit.clear();
    }

    pub fn set_message(&mut self, message: String) {
        self.message = message;
    }

    pub fn get_message(&self) -> &str {
        &self.message
    }

    pub fn is_open(&self) -> bool {
        self.is_open
    }
}