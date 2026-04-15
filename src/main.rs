// an implementation of the union file system from docker in rust
use std::{
    cell::RefCell,
    collections::{BTreeMap, HashMap},
    fs::{self, DirEntry, File, OpenOptions, read_dir},
    io,
    os::linux::fs::MetadataExt,
    path::{Path, PathBuf},
    time::SystemTime,
    vec,
};

use fuser::{Errno, FileType, INodeNo};
use rand::random;

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
            uid: 0, // do not use
            gid: 0, // do not use this
            blksize: BLOCK_SIZE,
            flags: 0,
            rdev: 0,                  // no clue what this is
            ctime: SystemTime::now(), // do not use this
        }
    }
}

struct UnionFS {
    base_readable_structure: String,
    next_file_handle: u64,
    next_process_id: u64,
    list_of_current_processes: HashMap<u64, process_state>, // matches a particular process to its respective process_state struct
}

// implementing the write only layer :
impl UnionFS {
    fn new(&self, base: String) -> UnionFS {
        UnionFS {
            base_readable_structure: base,
            next_file_handle: 1,
            next_process_id: 1,
            list_of_current_processes: HashMap::new(),
        }
    }

    fn new_proc(&mut self) {
        println!("The process id allocated : {}", self.next_process_id);
        let new_proc_state = process_state::new(self.next_process_id);
        self.list_of_current_processes
            .insert(self.next_process_id, new_proc_state);
        self.next_process_id += 1;
    }

    fn increment_file_handle(&mut self) {
        self.next_file_handle += 1;
    }
}

struct process_state {
    process_id: u64,                    //basically a container
    superBlock: u64,                    // the superBlock which allocates the new inode number
    file_mapping: HashMap<u64, String>, // mapping of an inode to the path in the writable path
    writable_path: String,
}

impl process_state {
    fn new(process_number_to_be_given: u64) -> process_state {
        println!(
            "new process with process id : {} created",
            process_number_to_be_given
        );
        let mut final_path = String::from("writable_path_");
        final_path += &process_number_to_be_given.to_string();
        let r: u64 = random();
        final_path += &r.to_string();
        process_state {
            process_id: process_number_to_be_given,
            superBlock: 1,
            file_mapping: HashMap::new(),
            writable_path: final_path,
        }
    }

    fn init_path(&mut self) {
        /*
                let root = InodeMetadata {
                    inode_val: INodeNo::ROOT.0,
                    open_file_handles: 0,
                    size: 0,
                    last_modified: SystemTime::now(),
                    last_accessed: SystemTime::now(),
                    last_metadata_change: SystemTime::now(),
                    kind: FileKind::Directory,
                    mode: 0o777,
                    hard_links: 2,
                };
        */
        let mut path = self.writable_path.clone();
        let tmp = String::from("/");
        path += &tmp;

        self.allocate_dir(path);
    }

    fn increment_superblock_val(&mut self) {
        self.superBlock += 1;
    }

    fn add_to_file_mapping(&mut self, new_file_inode: u64, path_of_file: String) {
        self.file_mapping.insert(new_file_inode, path_of_file);
    }

    fn allocate_dir(&mut self, path: String) {
        let mut comp_path = PathBuf::from(&self.writable_path);
        comp_path.push(path);
        OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(comp_path)
            .unwrap();
    }

    fn get_dir_content(&self, path: String) -> Result<Vec<String>, Errno> {
        let path = PathBuf::from(path);
        let mut v: Vec<String> = Vec::new();
        if path.is_dir() {
            let dir_iter = fs::read_dir(path).unwrap();
            for val in dir_iter {
                let val = val.unwrap();
                let file_name = val.file_name().into_string().unwrap();
                v.push(file_name);
            }
            return Ok(v);
        } else {
            return Err(fuser::Errno::ENOENT);
        }
    }

    // makes a new file, adds the inode and path to the hash map and returns the inode_val
    fn allocate_file(&mut self, path: String) -> u64 {
        let mut comp_path = PathBuf::from(&self.writable_path);
        comp_path.push(path);
        let file = OpenOptions::new()
            .write(true)
            .create(true)
            .open(&comp_path)
            .unwrap();
        let tmp = file.metadata().unwrap();
        self.add_to_file_mapping(tmp.st_ino(), comp_path.display().to_string());
        tmp.st_ino()
    }
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
    dir_vec: &mut Vec<String>,
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
    let mut readable_b_tree: BTreeMap<Vec<u8>, (u64, FileKind)> = BTreeMap::new();
    let all_files: RefCell<Vec<String>> = RefCell::new(vec![]);

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
        readable_b_tree.insert(test.as_bytes().to_vec(), (inode_number, FileKind::file));
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
        readable_b_tree.insert(
            v_dirs.as_bytes().to_vec(),
            (inode_number, FileKind::Directory),
        );
    }

    // need to make a mapping from the readable inode to writable inode
    let mappings: HashMap<u64, u64> = HashMap::new();
}
