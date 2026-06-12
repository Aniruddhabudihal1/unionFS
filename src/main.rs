use fuser::{Config, Errno, FileAttr, FileType, Filesystem, Generation, INodeNo, MountOption};
use std::{
    collections::HashMap,
    env,
    fs::{self},
    path::PathBuf,
    sync::{LockResult, RwLock, RwLockReadGuard, RwLockWriteGuard},
    time::{Duration, SystemTime},
};

#[derive(Debug)]
enum Node {
    Directory {
        hash_of_children: RwLock<HashMap<String, u64>>,
    },
    File {
        file_content: String,
    },
}

impl Node {
    fn just_access_children_without_editing_anything(
        &self,
    ) -> Option<LockResult<RwLockReadGuard<HashMap<String, u64>>>> {
        match self {
            Self::Directory { hash_of_children } => Some(hash_of_children.read()),
            Self::File { .. } => None,
        }
    }

    fn actual_access_to_children(
        &mut self,
    ) -> Option<LockResult<RwLockWriteGuard<HashMap<String, u64>>>> {
        match self {
            Self::Directory { hash_of_children } => Some(hash_of_children.write()),
            Self::File { .. } => None,
        }
    }

    fn get_file_content(&self) -> Option<&String> {
        match self {
            Node::Directory { .. } => None,
            Node::File { file_content } => Some(file_content),
        }
    }
}

#[derive(Debug)]
struct InodeContent {
    inode_attributes: FileAttr,
    node_kind: Node,
}

impl InodeContent {
    fn get_inode_kind(&self) -> FileType {
        match &self.node_kind {
            Node::Directory { .. } => FileType::Directory,
            Node::File { .. } => FileType::RegularFile,
        }
    }
}

struct unionFS {
    primary_pathname: RwLock<PathBuf>,
    session_id_mapping: RwLock<HashMap<String, u64>>,
    mapping: RwLock<HashMap<u64, InodeContent>>,
    curr_inode_val: RwLock<u64>,
}

impl unionFS {
    fn new(primary_pathname: PathBuf) -> Self {
        let now = SystemTime::now();
        let root_attr = FileAttr {
            ino: INodeNo(1),
            size: 0,
            blocks: 0,
            atime: now,
            mtime: now,
            ctime: now,
            crtime: now,
            kind: FileType::Directory,
            perm: 0o755,
            nlink: 0,
            uid: 0,
            gid: 0,
            rdev: 0,
            blksize: 512,
            flags: 0,
        };

        let root = InodeContent {
            inode_attributes: root_attr,
            node_kind: Node::Directory {
                hash_of_children: HashMap::new().into(),
            },
        };

        let mut primary_mapping = HashMap::new();
        primary_mapping.insert(1, root);

        let session_id_maps: HashMap<String, u64> = HashMap::new();
        unionFS {
            primary_pathname: primary_pathname.to_path_buf().into(),
            mapping: RwLock::new(primary_mapping),
            session_id_mapping: RwLock::new(session_id_maps),
            curr_inode_val: RwLock::new(1),
        }
    }
    /*
    fn instantiate_for_a_session_id(&mut self, re: &Request, ses_id: String, rep: ReplyEntry) {
        let tmp1 = &self.primary_pathname;
        let v: Vec<PathBuf> = Vec::new();

        let tmp2 = self.next_inode_value.clone();
        self.next_inode_value += 1;

        let f = make_attribute(tmp2, false);

        let _ = self.session_id_mapping.borrow_mut().insert(ses_id, tmp2);
        let hm: HashMap<String, u64> = HashMap::new();
        let inode_instance = InodeContent {
            inode_attributes: f,
            node_kind: Node::Directory {
                hash_of_children: hm.into(),
            },
        };
        self.mapping.borrow_mut().insert(tmp2, inode_instance);

        let _ = instantiate_fs(self, tmp1.to_path_buf(), v, tmp2, true);
    }
    */

    fn strip(&self, name: PathBuf) -> String {
        let base_path = self
            .primary_pathname
            .read()
            .unwrap()
            .to_path_buf()
            .to_string_lossy()
            .to_string();
        let inp = name.to_string_lossy().to_string();
        inp.replace(&base_path, "")
    }
}

fn increment_global_inode_val(mut val: RwLockWriteGuard<u64>) {
    *val += 1;
    drop(val);
}

fn identify_session_id(pid: String) -> String {
    let mut session_id_path: String = String::from("/proc/");
    session_id_path.push_str(&pid);
    session_id_path.push_str("/stat");
    let con = String::new();
    let c_i = con.split(" ");
    let mut tmp = 1;
    let mut session_id = String::new();
    for i in c_i {
        if tmp == 6 {
            println!(" this : {}", i);
            session_id = i.to_string().clone();
            break;
        }
        tmp += 1;
    }
    session_id
}

impl Filesystem for unionFS {
    fn lookup(
        &self,
        _req: &fuser::Request,
        parent: INodeNo,
        name: &std::ffi::OsStr,
        reply: fuser::ReplyEntry,
    ) {
        let name_local_format = name.to_string_lossy().to_string();
        println!(
            "lookup function got invoked for : {} and its parent inode value is : {}",
            name_local_format, parent.0
        );

        println!("The content in the parent is : ");

        let it = self.mapping.read().unwrap();

        for i in it.iter() {
            println!();
            println!("{:?}", i);
        }

        let global_instance = self.mapping.read().unwrap();
        let parent_content = global_instance
            .get(&parent.0)
            .unwrap()
            .node_kind
            .just_access_children_without_editing_anything()
            .unwrap()
            .unwrap();

        let child = parent_content.get(&name_local_format);
        match child {
            Some(i) => {
                let glob = self.mapping.read().unwrap();
                let child_actual = glob.get(i).unwrap().inode_attributes;
                let dur = Duration::default();
                reply.entry(&dur, &child_actual, Generation(1));
            }
            None => {
                println!("{} Does not exist", name_local_format);
                reply.error(Errno::ENOENT);
            }
        }
    }

    fn readdir(
        &self,
        _req: &fuser::Request,
        ino: INodeNo,
        _fh: fuser::FileHandle,
        offset: u64,
        mut reply: fuser::ReplyDirectory,
    ) {
        println!(
            "The inode number on which we are performing the readdir is : {}",
            ino.0
        );

        let global_hash_instance = self.mapping.read().unwrap();
        let tmp = global_hash_instance.iter().clone();

        let mut aggregate: Vec<(u64, FileType, String)> = Vec::new();

        for (i, ii) in tmp {
            let ft = ii.get_inode_kind();

            //println!("when inserting into ")
            aggregate.push((*i, ft, i.to_string()));
        }
        aggregate.push((ino.0, FileType::Directory, ".".to_string()));
        aggregate.push((ino.0, FileType::Directory, "..".to_string()));

        for (i, entry) in aggregate.into_iter().enumerate().skip(offset as usize) {
            println!("hello");
            if reply.add(INodeNo(entry.0), (i + 1) as u64, entry.1, entry.2) {
                break;
            }
        }
        reply.ok();
    }

    fn getattr(
        &self,
        _req: &fuser::Request,
        ino: INodeNo,
        _fh: Option<fuser::FileHandle>,
        reply: fuser::ReplyAttr,
    ) {
        println!("getattr got invoked for inode value : {}", ino.0);
        let d = Duration::default();
        if let Some(in_content) = self.mapping.read().unwrap().get(&ino.0) {
            reply.attr(&d, &in_content.inode_attributes);
        } else {
            reply.error(Errno::ENOENT);
        }
    }

    fn setattr(
        &self,
        _req: &fuser::Request,
        ino: INodeNo,
        _mode: Option<u32>,
        uid: Option<u32>,
        gid: Option<u32>,
        size: Option<u64>,
        _atime: Option<fuser::TimeOrNow>,
        _mtime: Option<fuser::TimeOrNow>,
        _ctime: Option<SystemTime>,
        _fh: Option<fuser::FileHandle>,
        _crtime: Option<SystemTime>,
        _chgtime: Option<SystemTime>,
        _bkuptime: Option<SystemTime>,
        flags: Option<fuser::BsdFileFlags>,
        reply: fuser::ReplyAttr,
    ) {
        let mut at = make_attribute(ino.0, false);
        at.flags = flags.unwrap().bits();
        at.size = size.unwrap();
        at.uid = uid.unwrap();
        at.gid = gid.unwrap();
        let dur = Duration::default();
        reply.attr(&dur, &at);
    }
}

fn make_attribute(inode_val: u64, dir: bool) -> FileAttr {
    let now = SystemTime::now();

    if dir {
        FileAttr {
            ino: INodeNo(inode_val),
            size: 0,
            blocks: 0,
            atime: now,
            mtime: now,
            ctime: now,
            crtime: now,
            kind: FileType::Directory,
            perm: 0o755,
            nlink: 0,
            uid: 0,
            gid: 0,
            rdev: 0,
            blksize: 512,
            flags: 0,
        }
    } else {
        FileAttr {
            ino: INodeNo(inode_val),
            size: 0,
            blocks: 0,
            atime: now,
            mtime: now,
            ctime: now,
            crtime: now,
            kind: FileType::RegularFile,
            perm: 0o755,
            nlink: 0,
            uid: 0,
            gid: 0,
            rdev: 0,
            blksize: 512,
            flags: 0,
        }
    }
}

// the goal of this is to recursively search for all the children of the directories and then add
// them to the vector
//
// each directory has a hash node mapping of : name (string) -> inode value (u64)
//
// then in the global state of the file system there is a mapping of : inode value (u64) -> InodeContent

fn instantiate_fs(
    file_system_instance: &unionFS,
    path: &PathBuf,
    parent_inode_value: u64,
    _do_only_dir: bool,
) {
    let dir_iter = fs::read_dir(path).unwrap();
    println!("current path is : {}", path.to_string_lossy());

    let parent_val = parent_inode_value.clone();
    {
        let next_val = file_system_instance.curr_inode_val.try_write().unwrap();
        increment_global_inode_val(next_val);
    }
    // write lock dropped here

    for dir_instance in dir_iter {
        let pathh = dir_instance.unwrap().path();
        let str_path = file_system_instance.strip(pathh.clone());

        // need to add this child of the parent in 2 hash maps :
        // parents mapping of : name -> inode value
        // global mapping of inode value -> InodeContent

        {
            if pathh.is_file() {
                increment_global_inode_val(
                    file_system_instance.curr_inode_val.try_write().unwrap(),
                );
            }
            let mut global_state = file_system_instance.mapping.try_write().unwrap();

            if let Some(parent_inodeinstance) = global_state.get_mut(&parent_val) {
                let mut parent_hash = parent_inodeinstance
                    .node_kind
                    .actual_access_to_children()
                    .unwrap()
                    .unwrap();

                println!(
                    "The name of the thing being inserted : {} and the inode value being given to it is : {}",
                    str_path.clone(),
                    *file_system_instance.curr_inode_val.read().unwrap()
                );

                parent_hash.insert(
                    str_path.clone(),
                    *file_system_instance.curr_inode_val.read().unwrap(),
                );
            } else {
                println!("parent does not exist");
            }
        }
        // write lock dropped here

        if pathh.is_file() {
            println!("file detected");
            let file_attr =
                make_attribute(*file_system_instance.curr_inode_val.read().unwrap(), false);

            let actual_content = fs::read_to_string(&pathh).unwrap();

            let new_node = InodeContent {
                inode_attributes: file_attr,
                node_kind: Node::File {
                    file_content: actual_content,
                },
            };

            {
                let second_global_state = file_system_instance.mapping.try_write();
                match second_global_state {
                    Ok(mut r) => {
                        r.insert(
                            *file_system_instance.curr_inode_val.read().unwrap(),
                            new_node,
                        );
                        drop(r);
                    }
                    Err(er) => {
                        println!(
                            "An error ocurred when trying to add file to the mapping\n{}",
                            er
                        );
                    }
                }
            }
            // write lock dropped here
        } else if pathh.is_dir() {
            println!("Directory detected with name : {:?}", pathh.clone());
            println!("And the path name is {}", str_path.clone());
            let file_attr =
                make_attribute(*file_system_instance.curr_inode_val.read().unwrap(), true);

            let new_hm: HashMap<String, u64> = HashMap::new();

            let new_node = InodeContent {
                inode_attributes: file_attr,
                node_kind: Node::Directory {
                    hash_of_children: RwLock::new(new_hm),
                },
            };

            {
                let second_global_state = file_system_instance.mapping.try_write();
                match second_global_state {
                    Ok(mut t) => {
                        println!("ok now it inserted and its no longer poisoned");
                        t.insert(
                            *file_system_instance.curr_inode_val.read().unwrap(),
                            new_node,
                        );
                        drop(t);
                    }
                    Err(er) => {
                        println!("the error is this : {}", er);
                    }
                }
            }
            // write dropped here

            println!(
                "the inode value being inserted is : {}",
                file_system_instance.curr_inode_val.read().unwrap().clone()
            );
            //input_vec.push(pathh.clone());
            let tmp = file_system_instance.curr_inode_val.read().unwrap().clone();
            instantiate_fs(file_system_instance, &pathh.clone(), tmp, false);
        }
    }
}

fn main() {
    let cmdline_args: Vec<String> = env::args().collect();
    let pathToBeMounted = &cmdline_args[2];
    println!("The path to be mounted is : {}", pathToBeMounted);
    let pathname: PathBuf = PathBuf::from(pathToBeMounted);

    let fileSystem_instance = unionFS::new(pathname.clone());

    if pathname.is_dir() {
        instantiate_fs(&fileSystem_instance, &pathname, 1, false);
    }

    let mut cfg = Config::default();
    let mut v = vec![MountOption::RW, MountOption::AutoUnmount];
    cfg.mount_options = v;
    cfg.acl = fuser::SessionACL::All;
    cfg.n_threads = Some(1);
    cfg.clone_fd = false;

    fuser::mount2(fileSystem_instance, pathname.clone(), &cfg).unwrap();
}
