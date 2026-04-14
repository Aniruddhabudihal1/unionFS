// an implementation of the union file system from docker in rust
use std::{
    cell::RefCell,
    collections::BTreeMap,
    fs::{self, DirEntry, File, read_dir},
    io,
    os::{linux::fs::MetadataExt, unix::fs::MetadataExt},
    path::Path,
    sync::atomic::AtomicU64,
    time::SystemTime,
    vec,
};

use fuser::{Errno, INodeNo};

const BLOCK_SIZE: u32 = 4096;

enum FileKind {
    file,
    Directory,
}

impl From<FileKind> for fuser::FileType {
    fn from(value: FileKind) -> Self {
        match value {
            FileKind::file => fuser::FileType::RegularFile,
            FileKind::Directory => fuser::FileType::Directory,
        }
    }
}

struct InodeMetadata {
    inode_val: u64,
    open_file_handles: u64,
    size: u64,
    last_modified: SystemTime,
    last_accessed: SystemTime,
    last_metadata_change: SystemTime,
    kind: FileKind,
    mode: u16,
    hard_links: u32,
    uid: u32,
}

impl From<InodeMetadata> for fuser::FileAttr {
    fn from(attr: InodeMetadata) -> Self {
        fuser::FileAttr {
            ino: INodeNo(attr.inode_val),
            size: attr.size,
            blocks: attr.size.div_ceil(u64::from(BLOCK_SIZE)),
            crtime: SystemTime::now(),
            atime: SystemTime::now(),
            mtime: SystemTime::now(),
            kind: attr.kind.into(),
            perm: attr.mode,
            nlink: attr.hard_links,
            uid: attr.uid, // do not use
            gid: 0,        // do not use this
            blksize: BLOCK_SIZE,
            flags: 0,
            rdev: 0,                  // no clue what this is
            ctime: SystemTime::now(), // do not use this
        }
    }
}

// the name of the string stored should be the full path, and not just the file name because there
// can be files in different directories with the same name
//
// directories also the same analogy where you need to have the full path upto that point
type readable_fs_directory = BTreeMap<Vec<u8>, (u64, FileKind)>;

fn file_exists(btree: &BTreeMap<Vec<u8>, (u64, FileKind)>, file_name: &String) -> bool {
    btree.get(file_name.as_bytes()).is_some()
}

struct process_state {
    process_id: u64,                                    //basically a container
    superBlock: u64, // the superBlock which allocates the new inode number
    writable_files: BTreeMap<Vec<u8>, (u64, FileKind)>, // each writable file or directory under a particular process is stored here
}

impl process_state {
    fn new(&mut self, process_number_to_be_given: u64) -> process_state {
        println!(
            "new process with process id : {} created",
            process_number_to_be_given
        );
        process_state {
            process_id: process_number_to_be_given,
            superBlock: 1,
            writable_files: BTreeMap::new(),
        }
    }

    fn increment_superblock_val(&mut self) {
        self.superBlock += 1;
    }

    fn add_file_to_Btree(
        &mut self,
        file_name: String,
        file_type: String,
        inode_number: u64,
    ) -> Errno {
        let mut btree_instance = &mut self.writable_files;
        if file_type.to_lowercase() == "file" {
            btree_instance.insert(
                file_name.as_bytes().to_vec(),
                (inode_number, FileKind::file),
            );
            return Errno::EEXIST;
        } else if file_type.to_lowercase() == "directory" {
            btree_instance.insert(
                file_name.as_bytes().to_vec(),
                (inode_number, FileKind::Directory),
            );
            return Errno::EEXIST;
        }
        println!("can only add files and directories");
        Errno::EINVAL
    }

    fn delete_file_from_btree(&mut self, file_name: String) -> Errno {
        let btree_instance = &mut self.writable_files;
        let state = file_exists(btree_instance, &file_name);

        if !state {
            println!("file does not exist");
            return Errno::ENOENT;
        }
        let _ = btree_instance.remove(file_name.as_bytes()).unwrap();
        Errno::EEXIST
    }

    fn search(&self, name: &String) -> bool {
        file_exists(&self.writable_files, name)
    }
}

// contains the writable layers implementation
struct UnionFS {
    data_dir: String,
    next_file_handle: AtomicU64,
    next_process_id: u64,
}

// implementing the write only layer :
//
// read from the read only layer clone it and then add it here

impl UnionFS {
    fn new(&self, new_data_dir: String) -> UnionFS {
        UnionFS {
            data_dir: new_data_dir,
            next_file_handle: AtomicU64::from(1),
            next_process_id: 1,
        }
    }

    fn get_inode();
    fn content_part();
    fn metadata_part();
    fn get_directory_content();
    fn lookup_name();

    fn insert_copied_data();
}

fn visited_files(dir: &Path, cb: &dyn Fn(&mut DirEntry)) -> io::Result<()> {
    if dir.is_dir() {
        for entry in read_dir(dir)? {
            let mut entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                visited_files(&path, cb)?;
            } else {
                cb(&mut entry);
            }
        }
    }
    Ok(())
}

fn visited_dirs(
    dir: &Path,
    cb: &dyn Fn(&mut DirEntry),
    mut dir_vec: &mut Vec<String>,
) -> io::Result<()> {
    if dir.is_dir() {
        for entry in read_dir(dir)? {
            let mut entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                dir_vec.push(path.display().to_string());
                visited_files(&path, cb)?;
            } else {
                cb(&mut entry);
            }
        }
    }
    Ok(())
}

fn main() {
    // The underlying data on which I mount my file system, should be first added to the readable
    // btree, which will contain its name, inode number and what kind of file it is
    //
    // I access it by first taking an open call to the underlying data and then not close the file
    // descriptor, which still allows me to access the files, and any write call issued to these
    // will be redirected to the appropriate process based write path

    let path = Path::new("underlying_data"); // hardcode value for now
    let readable_part = File::open(path).unwrap();
    let mut readable_B_tree: BTreeMap<Vec<u8>, (u64, FileKind)> = BTreeMap::new();
    let mut all_files: RefCell<Vec<String>> = RefCell::new(vec![]);

    let _ = visited_files(path, &mut |visit| {
        let tmp = visit.path().clone().to_str().unwrap().to_string();
        &all_files.borrow_mut().push(tmp);
    });
    let tmp = all_files.borrow_mut();
    let all_file_iter = tmp.iter();
    for test in all_file_iter {
        let tmp = File::open(test);
        let x = tmp.unwrap();
        let tmp = x.metadata().unwrap();
        let inode_number = tmp.st_ino();
        //let file_type = tmp.file_type();
        readable_B_tree.insert(test.as_bytes().to_vec(), (inode_number, FileKind::file));
    }
    // all files from the readable b tree inserted
    let mut all_dirs: Vec<String> = Vec::new();

    let _ = visited_dirs(
        path,
        &mut |_| println!("going through the directries"),
        &mut all_dirs,
    );

    let visited_dirs_iter = all_dirs.iter();
    for v_dirs in visited_dirs_iter {
        let path = Path::new(v_dirs);
        let tmp = fs::metadata(path).unwrap();
        let inode_number = tmp.st_ino();
        readable_B_tree.insert(
            v_dirs.as_bytes().to_vec(),
            (inode_number, FileKind::Directory),
        );
    }
}
