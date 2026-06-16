use fuser::{Config, Errno, FileAttr, FileType, Filesystem, Generation, INodeNo, MountOption};
use procfs::process::Process;
use std::{
    collections::HashMap,
    env,
    fs::{self},
    path::PathBuf,
    sync::{RwLock, RwLockWriteGuard},
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
    fn children_of_directory(&self) -> Option<&RwLock<HashMap<String, u64>>> {
        match self {
            Self::Directory { hash_of_children } => Some(hash_of_children),
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
    session_id_mapping: RwLock<HashMap<i32, u64>>,
    inode_to_content_mapping: RwLock<HashMap<u64, InodeContent>>,
    inode_to_string_mapping: RwLock<HashMap<u64, String>>,
    curr_inode_val: RwLock<u64>,
    lookup_history: RwLock<Vec<String>>,
    dir_mapping: RwLock<Vec<u64>>,
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

        let mut secondary_mapping: HashMap<u64, String> = HashMap::new();
        secondary_mapping.insert(1, "/".to_string());

        let lookup_history: Vec<String> = Vec::new();

        let session_id_maps: HashMap<i32, u64> = HashMap::new();
        let dirs: Vec<u64> = Vec::new();

        unionFS {
            primary_pathname: primary_pathname.to_path_buf().into(),
            inode_to_content_mapping: RwLock::new(primary_mapping),
            inode_to_string_mapping: RwLock::new(secondary_mapping),
            session_id_mapping: RwLock::new(session_id_maps),
            curr_inode_val: RwLock::new(1),
            lookup_history: RwLock::new(lookup_history),
            dir_mapping: RwLock::new(dirs),
        }
    }

    fn instantiate_session_id(&self, writable_root_inode: u64, readable_children: Vec<u64>) {
        // readable path -> writable path
        let mut inode_mappings: HashMap<u64, u64> = HashMap::new();
        let mut to_be_removed: Vec<u64> = Vec::new();
        let v_iter = readable_children.iter();
        for i in v_iter {
            to_be_removed.push(*i);

            {
                increment_global_inode_val(self.curr_inode_val.try_write().unwrap());
            }

            let cur = *self.curr_inode_val.try_read().unwrap();
            inode_mappings.insert(*i, cur);
            let new_hm: RwLock<HashMap<String, u64>> = RwLock::new(HashMap::new());
            let new_inode = InodeContent {
                inode_attributes: make_attribute(*self.curr_inode_val.try_read().unwrap(), true),
                node_kind: Node::Directory {
                    hash_of_children: new_hm,
                },
            };
            {
                let mut global_writable_instance =
                    self.inode_to_content_mapping.try_write().unwrap();
                let content = global_writable_instance
                    .get_mut(&writable_root_inode)
                    .unwrap();

                let tmp = content.node_kind.children_of_directory().unwrap();
                let mut root_children = tmp.try_write().unwrap();
                let tmp2 = self.inode_to_string_mapping.try_read().unwrap();
                let name = tmp2.get(i).unwrap();

                root_children.insert(name.to_string(), cur);
            }

            {
                let mut global_writable_instance =
                    self.inode_to_content_mapping.try_write().unwrap();
                global_writable_instance.insert(cur, new_inode);
            }
        }

        let v_iter2 = to_be_removed.iter();
        let mut new_vec: Vec<u64> = Vec::new();

        // iterator through readable nodes hash_of_children
        for i in v_iter2 {
            {
                // hashmap of the particular child node
                let tmp = self.inode_to_content_mapping.try_read().unwrap();
                let tmp2 = tmp.get(i).unwrap();
                let tmp4 = tmp2
                    .node_kind
                    .children_of_directory()
                    .unwrap()
                    .try_read()
                    .unwrap();
                let v_iter2 = tmp4.clone().into_values();
                for ii in v_iter2 {
                    new_vec.push(ii);
                }
            }
            let val = inode_mappings.get(i).unwrap();
            {
                self.instantiate_session_id(*val, new_vec.clone());
            }
        }
    }

    /*
    fn session_id_instantiation(&self, ses_id: i32) {
        let tmp = self.session_id_mapping.try_read().unwrap();
        let writable_root_inode = tmp.get(&ses_id).unwrap();

        let mut tmp2 = self.inode_to_content_mapping.try_write().unwrap();
        let num: u64 = 1;
        let readable_root = tmp2.get(&num).unwrap();
        let writable_instance = tmp2.get(writable_root_inode).unwrap();
        let readable_root_iter = readable_root
            .node_kind
            .just_access_children_without_editing_anything()
            .unwrap()
            .unwrap();
        let tmp3 = readable_root_iter.iter();

        for i in tmp3 {
            let tmp4 = writable_instance
                .node_kind
                .actual_access_to_children()
                .unwrap()
                .unwrap();
        }
    }
    */
}

fn increment_global_inode_val(mut val: RwLockWriteGuard<u64>) {
    *val += 1;
    drop(val);
}

fn strip(parent_path: PathBuf, child_path: PathBuf) -> String {
    let str_parent = parent_path.to_string_lossy().to_string();
    let str_child = child_path.to_string_lossy().to_string();

    let tmp = str_child.replace(&str_parent, "");
    tmp.replace("/", "")
}

impl Filesystem for unionFS {
    /*
    fn lookup(
        &self,
        req: &fuser::Request,
        parent: INodeNo,
        name: &std::ffi::OsStr,
        reply: fuser::ReplyEntry,
    ) {
        let name_local_format = name.to_string_lossy().to_string();
        println!(
            "lookup function got invoked for : {} and its parent inode value is : {}",
            name_local_format, parent.0
        );

        let comm_pid = req.pid() as i32;

        if let Ok(proc) = Process::new(comm_pid) {
            if let Ok(stat) = proc.stat() {
                let session_id = stat.tty_nr;
                println!("The session id is : {}", session_id);
                let mut ses_maps = self.session_id_mapping.try_write().unwrap();

                if let Some(res) = ses_maps.get(&session_id) {
                    println!("session_id exists with inode val : {}", res);
                } else {
                    // if session_id does not exist in the hash map
                    {
                        let tmp = self.curr_inode_val.try_write().unwrap();
                        increment_global_inode_val(tmp);
                    }

                    let new_hm: HashMap<String, u64> = HashMap::new();

                    let new_root = *self.curr_inode_val.try_read().unwrap();
                    let mut content = InodeContent {
                        inode_attributes: make_attribute(new_root, true),
                        node_kind: Node::Directory {
                            hash_of_children: RwLock::new(new_hm),
                        },
                    };

                    // the root of the new sesions ids writable path has been inserted
                    //
                    // now I need to insert rest of the directories from the readable path
                    //
                    // when instantiating the readable path, I made a mapping of each parent inode
                    // vlaue and its corresponding childrens inode values
                    ses_maps.insert(session_id, new_root);

                    {
                        let mut global_mapping = self.inode_to_content_mapping.try_write().unwrap();
                        global_mapping.insert(new_root, content);
                    }
                }

                {
                    let num = 1;
                    // writable inode -> readable inode
                    let mut hm: HashMap<u64, u64> = HashMap::new();
                    let writable_root_instance = ses_maps.get(&session_id).unwrap();
                    let mut temp = self
                        .inode_to_content_mapping
                        .try_write()
                        .unwrap()
                        .get_mut(&num)
                        .unwrap();
                    let writable_instance = temp.get_mut(writable_root_instance).unwrap();
                    let mut writable_map = writable_instance
                        .node_kind
                        .children_of_directory()
                        .unwrap()
                        .try_write()
                        .unwrap();

                    let readable_ins = temp.iter().clone();

                    for (name, ino) in readable_iter {
                        let tmp = temp.get(ino).unwrap();
                        if tmp.get_inode_kind() == FileType::Directory {
                            increment_global_inode_val(self.curr_inode_val.try_write().unwrap());
                            let cur_ino = self.curr_inode_val.try_read().unwrap();
                            hm.insert(*cur_ino, *ino);
                            writable_map.insert(name.to_string(), *cur_ino);

                            let new_hm: HashMap<String, u64> = HashMap::new();
                            let tmp2 = InodeContent {
                                inode_attributes: make_attribute(*cur_ino, true),
                                node_kind: Node::Directory {
                                    hash_of_children: RwLock::new(new_hm),
                                },
                            };

                            let mut ttmp = self.inode_to_content_mapping.try_write().unwrap();
                            ttmp.insert(*cur_ino, tmp2);
                        }
                    }
                }
            } else {
                println!("This process does not have a session id");
            }
        } else {
            println!("something went wrong when reading from procfs for the particular process");
        }

        {
            println!("helllo");
            let tmp = self.inode_to_content_mapping.try_read().unwrap();
            let tmp = tmp.iter();
            for i in tmp {
                println!();
                println!("{:?}", i);
                let ttmp = i.1.get_inode_kind();
                if ttmp == FileType::Directory {}
            }
        }

        let global_instance = self.inode_to_content_mapping.read().unwrap();
        let parent_content = global_instance
            .get(&parent.0)
            .unwrap()
            .node_kind
            .children_of_directory()
            .unwrap()
            .try_read()
            .unwrap();

        let child = parent_content.get(&name_local_format);
        match child {
            Some(i) => {
                println!(
                    "found the inode value for : {} which is {}",
                    name_local_format, i
                );
                let glob = self.inode_to_content_mapping.read().unwrap();
                let child_actual = glob.get(i).unwrap().inode_attributes;
                let dur = Duration::default();
                {
                    println!("checking the lookup history now");
                    let mut history_instance = self.lookup_history.try_write().unwrap();
                    if name != ".Trash"
                        || name != ".Trash-1000"
                        || name != ".xdg-volume-info"
                        || name != "autorun.inf"
                    {
                        history_instance.push(name_local_format.clone());
                        let h_iter = history_instance.iter();
                        for i in h_iter {
                            println!("{}", i);
                        }
                    }
                }
                reply.entry(&dur, &child_actual, Generation(1));
            }
            None => {
                println!("{} Does not exist", name_local_format);
                reply.error(Errno::ENOENT);
            }
        }
    }
    */

    fn lookup(
        &self,
        req: &fuser::Request,
        parent: INodeNo,
        name: &std::ffi::OsStr,
        reply: fuser::ReplyEntry,
    ) {
        let str_name = name.to_string_lossy().to_string();
        let comm_pid = req.pid() as i32;
        println!("The request pid is : {}", comm_pid);

        if let Ok(proc) = Process::new(comm_pid) {
            if let Ok(stat) = proc.stat() {
                let session_id = stat.tty_nr;
                println!("The session id is : {}", session_id);
                let mut ses_maps = self.session_id_mapping.try_write().unwrap();

                if let Some(res) = ses_maps.get(&session_id) {
                    println!("session_id exists with inode val : {}", res);
                } else {
                    println!("session id does not exist and we have to now add it to the hash map");
                    // have to instantiate session id
                    {
                        increment_global_inode_val(self.curr_inode_val.try_write().unwrap());
                    }
                    let cur = *self.curr_inode_val.try_read().unwrap();
                    ses_maps.insert(session_id, cur);
                    let new_hm: HashMap<String, u64> = HashMap::new();
                    let mut dir_children: Vec<u64> = Vec::new();

                    let new_root = *self.curr_inode_val.try_read().unwrap();
                    let content = InodeContent {
                        inode_attributes: make_attribute(new_root, true),
                        node_kind: Node::Directory {
                            hash_of_children: RwLock::new(new_hm),
                        },
                    };

                    {
                        let mut global_state = self.inode_to_content_mapping.try_write().unwrap();
                        global_state.insert(cur, content);
                        let num = 1;
                        let root_hash = global_state
                            .get(&num)
                            .unwrap()
                            .node_kind
                            .children_of_directory()
                            .unwrap()
                            .try_read()
                            .unwrap();

                        let r_iter = root_hash.clone().into_values();
                        for i in r_iter {
                            dir_children.push(i);
                        }
                    }

                    let mut dir_children_2: Vec<u64> = Vec::new();

                    {
                        let v_iter = dir_children.iter();
                        for i in v_iter {
                            let global_state = self.inode_to_content_mapping.try_read().unwrap();
                            let tmp = global_state.get(i).unwrap().get_inode_kind();
                            if tmp == FileType::Directory {
                                dir_children_2.push(*i);
                            }
                        }
                    }

                    {
                        //let tmp_writable = self.inode_to_content_mapping.try_write().unwrap();
                        // new function should take into it as input the base readable path from
                        // which it will read the directory and its children recursively and hence
                        // for that it will need readable instance of the inode to content mapping
                        //
                        // whereas the writable session id instance need to take the result of the
                        // readable paths InodeContent (just the directories) and then write it
                        // accordingly in the writable paths nodes
                        self.instantiate_session_id(cur, dir_children_2);
                    }
                }
            } else {
                println!("was not able to get session id");
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

        {
            let mut h_ins = self.lookup_history.try_write().unwrap();
            h_ins.clear();
        }

        let global_instance = self.inode_to_content_mapping.try_read().unwrap();
        let node_instance = global_instance
            .get(&ino.0)
            .unwrap()
            .node_kind
            .children_of_directory()
            .unwrap();
        let tmp2 = node_instance.try_read().unwrap();

        let tmp3 = tmp2.iter();

        let mut aggregate: Vec<(u64, FileType, String)> = Vec::new();

        for (i, ii) in tmp3 {
            let ft = global_instance.get(&ii).unwrap().get_inode_kind();

            aggregate.push((*ii, ft, i.to_string()));
        }
        aggregate.push((ino.0, FileType::Directory, ".".to_string()));
        aggregate.push((ino.0, FileType::Directory, "..".to_string()));

        for (i, entry) in aggregate.into_iter().enumerate().skip(offset as usize) {
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
        if let Some(in_content) = self.inode_to_content_mapping.read().unwrap().get(&ino.0) {
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
    do_only_dir: bool,
) {
    let parent_val = parent_inode_value;
    {
        let next_val = file_system_instance.curr_inode_val.try_write().unwrap();
        increment_global_inode_val(next_val);
    }

    let tmp = fs::read_dir(path);
    match tmp {
        Ok(dir_iter) => {
            for dir_instance in dir_iter {
                let pathh = dir_instance.unwrap().path();
                let str_child_path = strip(path.to_path_buf(), pathh.to_path_buf());

                // need to add this child of the parent in 2 hash maps :
                // parents mapping of : name -> inode value
                // global mapping of inode value -> InodeContent

                {
                    if pathh.is_file() && !do_only_dir {
                        increment_global_inode_val(
                            file_system_instance.curr_inode_val.try_write().unwrap(),
                        );
                    }
                    let mut global_state = file_system_instance
                        .inode_to_content_mapping
                        .try_write()
                        .unwrap();

                    let mut secondary_global_state = file_system_instance
                        .inode_to_string_mapping
                        .try_write()
                        .unwrap();
                    secondary_global_state.insert(
                        *file_system_instance.curr_inode_val.read().unwrap(),
                        str_child_path.clone(),
                    );

                    if let Some(parent_inodeinstance) = global_state.get_mut(&parent_val) {
                        let mut parent_hash = parent_inodeinstance
                            .node_kind
                            .children_of_directory()
                            .unwrap()
                            .try_write()
                            .unwrap();

                        println!(
                            "The name of the thing being inserted : {} and the inode value being given to it is : {}",
                            str_child_path.clone(),
                            *file_system_instance.curr_inode_val.read().unwrap()
                        );

                        parent_hash.insert(
                            str_child_path.clone(),
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
                        let second_global_state =
                            file_system_instance.inode_to_content_mapping.try_write();
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
                    println!("And the path name is {}", str_child_path.clone());
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
                        let second_global_state =
                            file_system_instance.inode_to_content_mapping.try_write();
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
                    {
                        let mut tmp = file_system_instance.dir_mapping.try_write().unwrap();
                        let l = tmp.len();
                        let c = *file_system_instance.curr_inode_val.try_read().unwrap();
                        tmp.insert(l, c);
                    }
                    let tmp = *file_system_instance.curr_inode_val.read().unwrap();
                    instantiate_fs(file_system_instance, &pathh.clone(), tmp, false);
                }
            }
        }
        Err(er) => {
            println!("could not read from the directory cuz : {}", er);
        }
    }
    println!("current path is : {}", path.to_string_lossy());
}

fn main() {
    let cmdline_args: Vec<String> = env::args().collect();
    let pathToBeMounted = &cmdline_args[2];
    println!("The path to be mounted is : {}", pathToBeMounted);
    let pathname: PathBuf = PathBuf::from(pathToBeMounted);

    let mut fileSystem_instance = unionFS::new(pathname.clone());

    fileSystem_instance.primary_pathname = RwLock::new(pathname.clone());

    if pathname.is_dir() {
        instantiate_fs(&fileSystem_instance, &pathname, 1, false);
    }

    let mut cfg = Config::default();
    let v = vec![MountOption::RW, MountOption::AutoUnmount];
    cfg.mount_options = v;
    cfg.acl = fuser::SessionACL::All;
    cfg.n_threads = Some(1);
    cfg.clone_fd = false;

    fuser::mount2(fileSystem_instance, pathname.clone(), &cfg).unwrap();
}
