// an implementation of the union file system from docker in rust
use std::{
    cell::RefCell,
    collections::{BTreeMap, HashMap},
    fs::{self, DirEntry, File, OpenOptions, read_dir},
    io::{self, ErrorKind, Read},
    os::linux::fs::MetadataExt,
    path::{Path, PathBuf},
    time::SystemTime,
    vec,
};

use fuser::{Config, Errno, Filesystem, INodeNo, SessionACL};
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
    base_readable_path: String,
    next_file_handle: u64,
    next_process_id: u64,
    list_of_current_processes: HashMap<u64, process_state>, // matches a particular process to its respective process_state struct
    process_writing_path_mapping: HashMap<u64, String>,
    base_dirs: Vec<String>,
}

// implementing the write only layer :
impl UnionFS {
    fn new(base: String, readable_base_dirs: Vec<String>) -> UnionFS {
        UnionFS {
            base_readable_path: base,
            next_file_handle: 1,
            next_process_id: 1,
            list_of_current_processes: HashMap::new(),
            process_writing_path_mapping: HashMap::new(),
            base_dirs: readable_base_dirs,
        }
    }

    fn new_proc(&mut self) {
        println!("The process id allocated : {}", self.next_process_id);
        let new_proc_state = process_state::new(self.next_process_id);
        let tmp = &new_proc_state.writable_path.clone();
        self.list_of_current_processes
            .insert(self.next_process_id, new_proc_state);
        self.process_writing_path_mapping
            .insert(self.next_process_id, tmp.to_string());
        self.next_process_id += 1;
    }

    fn increment_file_handle(&mut self) {
        self.next_file_handle += 1;
    }

    // The below are functions which can be executed by each process associate with the unionFS
    // struct, and because each is a separate struct they are all separated
    //
    // the below functions can then be called by the trait implementation of the Filesystem

    fn initialize_before_first_write(&mut self, proc_id: u64) {
        let state = &mut self.list_of_current_processes;
        let proc_state = state.get_mut(&proc_id).unwrap();
        proc_state.init_path(self.base_dirs.clone());
    }

    fn attach_new_dir(&mut self, proc_id: u64, new_dir_path: String) -> Result<u64, Errno> {
        let state = &mut self.list_of_current_processes;
        let proc_state = state.get_mut(&proc_id).unwrap();

        let inode = proc_state.allocate_dir(new_dir_path);
        match inode {
            Ok(val) => Ok(val),
            Err(er) => Err(er),
        }
    }

    fn attach_new_file(&mut self, proc_id: u64, new_file_path: String) -> Result<u64, Errno> {
        let state = &mut self.list_of_current_processes;
        let proc_state = state.get_mut(&proc_id).unwrap();

        proc_state.allocate_file(new_file_path)
    }

    fn read_dir_content(&mut self, proc_id: u64, inode_val: u64) -> Result<Vec<String>, Errno> {
        let state = &mut self.list_of_current_processes;
        let proc_state = state.get_mut(&proc_id).unwrap();

        proc_state.get_dir_content(inode_val)
    }

    fn read_file_content(&mut self, proc_id: u64, inode_val: u64) -> Result<String, Errno> {
        let state = &mut self.list_of_current_processes;
        let proc_state = state.get_mut(&proc_id).unwrap();

        proc_state.get_file_content(inode_val)
    }
}

struct process_state {
    process_id: u64,                    //basically a container
    file_mapping: HashMap<u64, String>, // mapping of an inode to the path in the writable path
    dir_mapping: HashMap<u64, String>,
    writable_path: String,
}

impl process_state {
    fn new(process_number_to_be_given: u64) -> process_state {
        println!(
            "new process with process id : {} created",
            process_number_to_be_given
        );
        let mut final_path = String::from("/writable_path_");
        final_path += &process_number_to_be_given.to_string();
        let r: u64 = random();
        final_path += &r.to_string();
        process_state {
            process_id: process_number_to_be_given,
            file_mapping: HashMap::new(),
            dir_mapping: HashMap::new(),
            writable_path: final_path,
        }
    }

    fn init_path(&mut self, dirs: Vec<String>) {
        self.allocate_dir(self.writable_path.clone());
        let dir_iter = dirs.iter();
        for dir in dir_iter {
            self.allocate_dir(dir.to_string());
        }
    }

    fn add_to_file_mapping(&mut self, new_file_inode: u64, path_of_file: String) {
        self.file_mapping.insert(new_file_inode, path_of_file);
    }

    fn add_to_dir_mapping(&mut self, new_dir_inode: u64, path_of_file: String) {
        self.dir_mapping.insert(new_dir_inode, path_of_file);
    }

    fn allocate_dir(&mut self, path: String) -> Result<u64, Errno> {
        let mut comp_path = PathBuf::from(&self.writable_path);
        comp_path.push(path);
        let tmp = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&comp_path);
        if let Ok(file) = tmp {
            let met = file.metadata().unwrap();
            self.add_to_dir_mapping(met.st_ino(), comp_path.display().to_string());
            Ok(met.st_ino())
        } else if let Err(er_code) = tmp {
            match er_code.kind() {
                ErrorKind::AlreadyExists => Err(Errno::EEXIST),
                _ => Err(Errno::ENODATA),
            }
        } else {
            Err(Errno::ENODATA)
        }
    }

    fn get_dir_content(&self, inode_val: u64) -> Result<Vec<String>, Errno> {
        let check = self.dir_mapping.get(&inode_val);
        match check {
            Some(path_valid) => {
                let path = PathBuf::from(path_valid);
                let mut v: Vec<String> = Vec::new();
                let dir_iter = fs::read_dir(path).unwrap();
                for val in dir_iter {
                    let val = val.unwrap();
                    let file_name = val.file_name().into_string().unwrap();
                    v.push(file_name);
                }
                Ok(v)
            }
            None => Err(Errno::ENOENT),
        }
    }

    fn file_existance_check_based_on_inode_val(&self, inode_val: u64) -> bool {
        let tmp = &self.file_mapping;
        tmp.get(&inode_val).is_some()
    }

    // makes a new file, adds the inode and path to the hash map and returns the inode_val
    fn allocate_file(&mut self, path: String) -> Result<u64, Errno> {
        let mut comp_path = PathBuf::from(&self.writable_path);
        comp_path.push(path);
        let file_tmp = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&comp_path);

        if let Ok(file) = file_tmp {
            let tmp = file.metadata().unwrap();
            self.add_to_file_mapping(tmp.st_ino(), comp_path.display().to_string());
            Ok(tmp.st_ino())
        } else if let Err(er_code) = file_tmp {
            match er_code.kind() {
                ErrorKind::AlreadyExists => return Err(Errno::EEXIST),
                _ => return Err(Errno::ENODATA),
            }
        } else {
            Err(Errno::ENODATA)
        }
    }

    fn get_file_content(&self, inode_val: u64) -> Result<String, Errno> {
        let file_map = &self.file_mapping;
        match file_map.get(&inode_val) {
            Some(path_name) => {
                // file exists
                let mut tmp = File::open(path_name).unwrap();
                let mut content = String::new();
                let _ = tmp.read_to_string(&mut content);
                Ok(content)
            }
            _ => Err(Errno::ENOENT),
        }
    }
}

impl Filesystem for UnionFS {
    fn init(&mut self, _req: &fuser::Request, _config: &mut fuser::KernelConfig) -> io::Result<()> {
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

    let base_path = Path::new("underlying_data"); // hardcode value for now
    let mut readable_b_tree: BTreeMap<Vec<u8>, (u64, FileKind)> = BTreeMap::new();
    let all_files: RefCell<Vec<String>> = RefCell::new(vec![]);

    let _ = visited_files(base_path, &mut |visit| {
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
        base_path,
        &mut |_| println!("going through the directries"),
        &mut all_dirs,
    );

    let mut vec_dirs: Vec<String> = Vec::new();
    let visited_dirs_iter = all_dirs.iter();
    for v_dirs in visited_dirs_iter {
        let path = Path::new(v_dirs);
        let tmp = fs::metadata(&path).unwrap();
        let inode_number = tmp.st_ino();
        readable_b_tree.insert(
            v_dirs.as_bytes().to_vec(),
            (inode_number, FileKind::Directory),
        );
        vec_dirs.push(v_dirs.to_string());
    }

    // list of all processes and the mapping of the local readable file paths inodes to that
    // particular processes writable paths inode val so that whatever query is being done can use
    // that inode number to make the subsequent stuff in the writable path, that is if it is
    // needed, not on every instance
    //
    // process_number -> (readable inode value -> writable inode value)
    //
    // so first you check for the process numebr that you are using in the hash map and that is the
    // first level of check,
    // if process does not exist create a new process in the hash map and the second hash map
    // remains empty till whenever you decide to edit the file
    let mut list_of_pocesses: HashMap<u64, HashMap<u64, u64>> = HashMap::new();
    let test_process_number: u64 = 10;
    let mut mapper: HashMap<u64, u64> = HashMap::new();
    mapper.insert(12, 48);
    list_of_pocesses.insert(test_process_number, mapper);

    let final1 = fuser::mount2(
        UnionFS::new(base_path.display().to_string()),
        base_path.into(),
        &Config::default(),
    );
}
