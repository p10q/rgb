use std::path::PathBuf;

pub struct FileTreeComponent {
    root: PathBuf,
    expanded_dirs: Vec<PathBuf>,
    selected: Option<PathBuf>,
}

impl FileTreeComponent {
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            expanded_dirs: Vec::new(),
            selected: None,
        }
    }

    pub fn toggle_expand(&mut self, path: PathBuf) {
        if let Some(pos) = self.expanded_dirs.iter().position(|p| p == &path) {
            self.expanded_dirs.remove(pos);
        } else {
            self.expanded_dirs.push(path);
        }
    }

    pub fn select(&mut self, path: PathBuf) {
        self.selected = Some(path);
    }
}