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
    dir_mapping: RwLock<Vec<u64>>,
    writable_to_readable_inode: RwLock<HashMap<u64, Option<u64>>>,
    first_lookup: RwLock<bool>,
    writable_instance_of_parent: RwLock<Option<u64>>,
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

        let session_id_maps: HashMap<i32, u64> = HashMap::new();
        let dirs: Vec<u64> = Vec::new();

        let mapping_of_inodes: HashMap<u64, Option<u64>> = HashMap::new();

        let first_lookup: bool = true;

        // it is by design an Option because it will allow you to encode wether it should be
        // reading the writable instance of the parent
        let writable_parent_instance_inode_val: Option<u64> = None;

        unionFS {
            primary_pathname: primary_pathname.to_path_buf().into(),
            inode_to_content_mapping: RwLock::new(primary_mapping),
            inode_to_string_mapping: RwLock::new(secondary_mapping),
            session_id_mapping: RwLock::new(session_id_maps),
            curr_inode_val: RwLock::new(1),
            dir_mapping: RwLock::new(dirs),
            writable_to_readable_inode: RwLock::new(mapping_of_inodes),
            first_lookup: RwLock::new(first_lookup),
            writable_instance_of_parent: RwLock::new(writable_parent_instance_inode_val),
        }
    }

    fn instantiate_session_id(&self, writable_root_inode: u64, readable_children: Vec<u64>) {
        // readable path -> writable path
        let mut inode_mappings: HashMap<u64, u64> = HashMap::new();
        let mut to_be_removed: Vec<u64> = Vec::new();
        let v_iter = readable_children.iter();
        // The children of the readable root as passed into the function
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
                let mut writable_to_readable = self.writable_to_readable_inode.try_write().unwrap();
                writable_to_readable.insert(cur, Some(*i));
            }

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
    fn lookup(
        &self,
        req: &fuser::Request,
        parent: INodeNo,
        name: &std::ffi::OsStr,
        reply: fuser::ReplyEntry,
    ) {
        let dur = Duration::default();
        let str_name = name.to_string_lossy().to_string();
        let comm_pid = req.pid() as i32;
        println!("The request pid is : {}", comm_pid);
        let ses_id: i32;

        if let Ok(proc) = Process::new(comm_pid) {
            if let Ok(stat) = proc.stat() {
                ses_id = stat.tty_nr;
                println!("The session id is : {}", ses_id);
                let mut ses_maps = self.session_id_mapping.try_write().unwrap();

                if let Some(res) = ses_maps.get(&ses_id) {
                    println!("session_id exists with inode val : {}", res);
                } else {
                    println!("session id does not exist and we have to now add it to the hash map");
                    // have to instantiate session id
                    {
                        increment_global_inode_val(self.curr_inode_val.try_write().unwrap());
                    }
                    let cur = *self.curr_inode_val.try_read().unwrap();
                    ses_maps.insert(ses_id, cur);
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

                    // TODO : need to find a cleaner way of dealing with this, instead of
                    // instantiating a new vector
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
                        self.instantiate_session_id(cur, dir_children_2);
                    }
                }

                // actual lookup logic
                let writable_root_instance = ses_maps.get(&ses_id).unwrap();

                let mut is_it_the_first_lookup = self.first_lookup.try_write().unwrap();
                if *is_it_the_first_lookup {
                    // if its the first lookup being performed
                    {
                        *is_it_the_first_lookup = false;
                    }
                    let global_state = self.inode_to_content_mapping.try_read().unwrap();

                    let children_tmp = global_state.get(writable_root_instance).unwrap();
                    let children = children_tmp
                        .node_kind
                        .children_of_directory()
                        .unwrap()
                        .try_read()
                        .unwrap();

                    if let Some(state) = children.get(&str_name) {
                        // found teh node in the writable path
                        let final_state = global_state.get(state).unwrap();
                        let final_inodecontent = final_state.inode_attributes;

                        if final_state.get_inode_kind() == FileType::RegularFile {
                            *is_it_the_first_lookup = true;
                        }
                        reply.entry(&dur, &final_inodecontent, Generation(1));
                    } else {
                        // see if the node exists in the readable path
                        let tmp2 = global_state.get(&parent.0).unwrap();
                        let tmp3 = tmp2.node_kind.children_of_directory().unwrap();
                        let tmp4 = tmp3.try_read().unwrap();

                        if let Some(tmp5) = tmp4.get(&str_name) {
                            // exists in the readable path
                            //
                            // TODO : it would exist only in the readable path if it were a file
                            // and not a directory, so you technically dont need to hadle the edge
                            // case where it might or might not be a file
                            let final_inodecontent = global_state.get(tmp5).unwrap();
                            if final_inodecontent.get_inode_kind() == FileType::RegularFile {
                                *is_it_the_first_lookup = true;
                                let mut tmpp =
                                    self.writable_instance_of_parent.try_write().unwrap();
                                *tmpp = Some(*writable_root_instance);
                                // need to keep a global state of the corresponding parents
                                // writable instance so that if the file is being edited we can
                                // create a new copy for it in the writable path
                                reply.entry(
                                    &dur,
                                    &final_inodecontent.inode_attributes,
                                    Generation(1),
                                );
                            }
                        } else {
                            // exists in neither the readable nor the writable path
                            {
                                *is_it_the_first_lookup = true;
                            }
                            reply.error(Errno::ENOENT);
                        }
                    }
                } else {
                    // beyond first lookup
                    // checking in writable path for child
                    let global_inodecontent_instance =
                        self.inode_to_content_mapping.try_write().unwrap();
                    let parent_tmp = global_inodecontent_instance.get(&parent.0).unwrap();

                    let writable_parents_hashmap = parent_tmp
                        .node_kind
                        .children_of_directory()
                        .unwrap()
                        .try_read()
                        .unwrap();

                    if let Some(target_exists) = writable_parents_hashmap.get(&str_name) {
                        // it is present in the writable path
                        let inode_content =
                            global_inodecontent_instance.get(target_exists).unwrap();
                        reply.entry(&dur, &inode_content.inode_attributes, Generation(1));
                    } else {
                        // not present in the writable path
                        let tmp = self.writable_to_readable_inode.try_read().unwrap();
                        let corresponding_readable_inode = tmp.get(&parent.0).unwrap().unwrap();
                        let readable_path_inode = global_inodecontent_instance
                            .get(&corresponding_readable_inode)
                            .unwrap();
                        let readable_parent_hash_map = readable_path_inode
                            .node_kind
                            .children_of_directory()
                            .unwrap()
                            .try_write()
                            .unwrap();

                        if let Some(child) = readable_parent_hash_map.get(&str_name) {
                            // child exists in readable path but not in the writable path
                            let mut writable_parent_inode =
                                self.writable_instance_of_parent.try_write().unwrap();
                            *writable_parent_inode = Some(parent.0);

                            let child_inodeattri = global_inodecontent_instance
                                .get(child)
                                .unwrap()
                                .inode_attributes;
                            reply.entry(&dur, &child_inodeattri, Generation(1));
                        } else {
                            // child exists in neither readable nor writable path
                            let mut edit_state = self.first_lookup.try_write().unwrap();
                            *edit_state = true;

                            reply.error(Errno::ENOENT);
                        }
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
            let ft = global_instance.get(ii).unwrap().get_inode_kind();

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
