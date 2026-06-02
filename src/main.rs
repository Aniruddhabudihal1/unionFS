use std::{
    collections::HashMap,
    env,
    fs::{self},
    hash::Hash,
    path::PathBuf,
};

/*
enum Node {
    Directory {
        hash_of_children: HashMap<String, i32>,
    },
    File {
        file_content: String,
    },
}

struct InodeContent {
    Inode_attributes: FileAttribute,
    Node_kind: Node,
}

impl InodeContent {
    fn InodeKind(&self) -> FileKind {
        match self.Node_kind {
            Node::Directory { hash_of_children } => FileKind::Directory,
            Node::File { file_content } => FileKind::RegularFile,
        }
    }
}
*/

// have to peroform a .is_dir() on this before calling this
fn checking_recursively(
    base_pathname: PathBuf,
    mut pathname: PathBuf,
    mut list: Vec<PathBuf>,
) -> Vec<PathBuf> {
    let foo2 = fs::read_dir(&pathname);
    match foo2 {
        Ok(r) => {
            for bar in r {
                let test = &bar.unwrap().path().to_path_buf();
                if test.is_dir() {
                    list.push(test.strip_prefix(&base_pathname).unwrap().to_path_buf());
                    list = checking_recursively(base_pathname.clone(), test.to_path_buf(), list);
                }
            }
        }
        Err(_) => println!("check if the path is an actual directory"),
    }
    list
}

fn main() {
    // task 1 : making a clone of the directory we are mounting and keeping the state of it
    // task 2 : check if I can access the same mounted from multiple terminal windows and allow for
    // each to have the skeletal writable path

    let cmdline_args: Vec<String> = env::args().collect();
    let pathToBeMounted = &cmdline_args[2];
    println!("The path to be mounted is : {}", pathToBeMounted);
    let pathname: PathBuf = PathBuf::from(pathToBeMounted);

    let mut list_of_directories: Vec<PathBuf> = Vec::new();
    if pathname.is_dir() {
        list_of_directories = checking_recursively(pathname.clone(), pathname, list_of_directories);
        let l_iter = list_of_directories.iter();
        for i in l_iter {
            println!("{}", i.display());
        }
    }
}
