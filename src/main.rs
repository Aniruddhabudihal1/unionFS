use fuser::{
    Config, Errno, FileAttr, FileHandle, FileType, Filesystem, FopenFlags, Generation, INodeNo,
    MountOption,
};

use procfs::process::Process;
use std::{
    collections::HashMap,
    env, fs,
    path::PathBuf,
    sync::{RwLock, RwLockWriteGuard},
    time::{Duration, SystemTime},
};

#[derive(Debug)]
enum Node {
    Directory {
        hash_of_children: RwLock<HashMap<String, u64>>,
        writable_parent_instance: RwLock<HashMap<i32, u64>>,
        is_this_in_readable_path: bool,
        is_this_in_writable_path: bool,
    },
    File {
        file_content: RwLock<Vec<u8>>,
        is_this_in_readable_path: bool,
        is_this_in_writable_path: bool,
        writable_parent_instance: RwLock<HashMap<i32, u64>>,
    },
}

impl Node {
    fn children_of_directory(&self) -> Option<&RwLock<HashMap<String, u64>>> {
        match self {
            Self::Directory {
                hash_of_children, ..
            } => Some(hash_of_children),
            Self::File { .. } => None,
        }
    }

    fn get_file_content(&self) -> Option<&RwLock<Vec<u8>>> {
        match self {
            Node::Directory { .. } => None,
            Node::File { file_content, .. } => Some(file_content),
        }
    }

    fn update_writable_parent_instance_for_dirs(&self, ses_id: i32, writable_parent: u64) {
        match self {
            Self::Directory {
                writable_parent_instance,
                ..
            } => {
                println!("writable instance hm : {:?}", writable_parent_instance);
                writable_parent_instance
                    .try_write()
                    .unwrap()
                    .insert(ses_id, writable_parent);
            }
            Self::File { .. } => {}
        }
    }

    fn get_writable_parent_instance_for_dirs(&self, ses_id: i32) -> Option<u64> {
        match self {
            Self::Directory {
                writable_parent_instance,
                ..
            } => {
                println!(
                    "getting writbale parent instance hm : {:?}",
                    writable_parent_instance.try_read().unwrap()
                );
                Some(
                    *writable_parent_instance
                        .try_read()
                        .unwrap()
                        .get(&ses_id)
                        .unwrap(),
                )
            }
            Self::File { .. } => None,
        }
    }

    fn update_writable_parent_instance_for_files(&self, ses_id: i32, writable_parent: u64) {
        match self {
            Self::Directory { .. } => {}
            Self::File {
                writable_parent_instance,
                ..
            } => {
                writable_parent_instance
                    .try_write()
                    .unwrap()
                    .insert(ses_id, writable_parent);
            }
        }
    }

    fn get_writable_parent_instance_for_files(&self, ses_id: i32) -> Option<u64> {
        match self {
            Self::Directory { .. } => None,
            Self::File {
                writable_parent_instance,
                ..
            } => Some(
                *writable_parent_instance
                    .try_read()
                    .unwrap()
                    .get(&ses_id)
                    .unwrap(),
            ),
        }
    }

    fn is_it_present_in_readable_path(&self) -> Option<bool> {
        match self {
            Node::Directory {
                is_this_in_readable_path,
                ..
            } => Some(*is_this_in_readable_path),
            Node::File {
                is_this_in_readable_path,
                ..
            } => Some(*is_this_in_readable_path),
        }
    }

    fn is_it_present_in_writable_path(&self) -> Option<bool> {
        match self {
            Node::Directory {
                is_this_in_writable_path,
                ..
            } => Some(*is_this_in_writable_path),
            Node::File {
                is_this_in_writable_path,
                ..
            } => Some(*is_this_in_writable_path),
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

struct UnionFs {
    primary_pathname: RwLock<PathBuf>,
    session_id_mapping: RwLock<HashMap<i32, u64>>,
    inode_to_content_mapping: RwLock<HashMap<u64, InodeContent>>,
    inode_to_string_mapping: RwLock<HashMap<u64, String>>,
    curr_inode_val: RwLock<u64>,
    writable_to_readable_inode: RwLock<HashMap<u64, Option<u64>>>,
    curr_file_handle: RwLock<u64>,
}

impl UnionFs {
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
                writable_parent_instance: RwLock::new(HashMap::new()),
                is_this_in_readable_path: true,
                is_this_in_writable_path: false,
            },
        };

        let mut primary_mapping = HashMap::new();
        primary_mapping.insert(1, root);

        let mut secondary_mapping: HashMap<u64, String> = HashMap::new();
        secondary_mapping.insert(1, "/".to_string());

        let session_id_maps: HashMap<i32, u64> = HashMap::new();

        let mapping_of_inodes: HashMap<u64, Option<u64>> = HashMap::new();

        UnionFs {
            primary_pathname: primary_pathname.to_path_buf().into(),
            inode_to_content_mapping: RwLock::new(primary_mapping),
            inode_to_string_mapping: RwLock::new(secondary_mapping),
            session_id_mapping: RwLock::new(session_id_maps),
            curr_inode_val: RwLock::new(1),
            writable_to_readable_inode: RwLock::new(mapping_of_inodes),
            curr_file_handle: RwLock::new(0),
        }
    }

    fn clone_fs(&self, writable_root_inode: u64, ses_id: i32) {
        let mut readable_equivalent_of_root: Option<u64> = None;
        {
            let tmp = self.writable_to_readable_inode.try_read().unwrap();
            readable_equivalent_of_root = Some(tmp.get(&writable_root_inode).unwrap().unwrap());
        }

        let mut parent_name: Option<&String> = None;

        let mut readable_children_vector: Vec<u64> = Vec::new();

        {
            let readable_content = self.inode_to_content_mapping.try_read().unwrap();
            let readable_instance = readable_content
                .get(&readable_equivalent_of_root.unwrap())
                .unwrap();
            let readable_children = readable_instance
                .node_kind
                .children_of_directory()
                .unwrap()
                .try_read()
                .unwrap();
            println!("printing readable children hm : {:?}", readable_children);

            let readable_children_iter = readable_children.iter();

            let tmpp = self.inode_to_string_mapping.try_read().unwrap();

            parent_name = tmpp.get(&writable_root_inode);

            for (_, ii) in readable_children_iter {
                println!("invoking ii for : {}", *ii);
                let readable_instance = readable_content.get(ii).unwrap();
                let readable_type = readable_instance.get_inode_kind();
                if readable_type == FileType::Directory {
                    println!(
                        "We are pushing {} which has parent {:?} and session id : {} and its respective writable_root_inode is : {}",
                        ii, parent_name, ses_id, writable_root_inode
                    );
                    readable_children_vector.push(*ii);
                    readable_instance
                        .node_kind
                        .update_writable_parent_instance_for_dirs(ses_id, writable_root_inode);
                } else if readable_type == FileType::RegularFile {
                    readable_content
                        .get(ii)
                        .unwrap()
                        .node_kind
                        .update_writable_parent_instance_for_files(ses_id, writable_root_inode);
                }
            }
        }

        let readable_children_iter = readable_children_vector.iter();

        for i in readable_children_iter {
            {
                increment_global_inode_val(self.curr_inode_val.try_write().unwrap());
            }

            let mut name: Option<String> = None;
            let new_writable_inode_val = *self.curr_inode_val.try_read().unwrap();
            {
                let global_state = self.inode_to_content_mapping.try_write().unwrap();
                let writable_instance = global_state.get(&writable_root_inode).unwrap();
                let mut writable_hm = writable_instance
                    .node_kind
                    .children_of_directory()
                    .unwrap()
                    .try_write()
                    .unwrap();

                let global_inode_to_name_map = self.inode_to_string_mapping.try_write().unwrap();
                name = Some(global_inode_to_name_map.get(i).unwrap().clone());
                println!(
                    "{} was inserted with the inode : {}",
                    name.as_ref().unwrap(),
                    new_writable_inode_val
                );

                // this is the writable hash map of the parent to which we are adding the name and
                // its respective inode value
                writable_hm.insert(name.as_ref().unwrap().to_string(), new_writable_inode_val);
            }

            {
                let mut inode_to_name = self.inode_to_string_mapping.try_write().unwrap();
                inode_to_name.insert(new_writable_inode_val, name.unwrap().clone());
            }

            {
                let mut writable_to_readable_mapping =
                    self.writable_to_readable_inode.try_write().unwrap();
                writable_to_readable_mapping.insert(new_writable_inode_val, Some(*i));
            }

            {
                let mut global_state = self.inode_to_content_mapping.try_write().unwrap();
                let new_writable_hm: HashMap<String, u64> = HashMap::new();
                let new_content = InodeContent {
                    inode_attributes: make_attribute(new_writable_inode_val, true),
                    node_kind: Node::Directory {
                        hash_of_children: RwLock::new(new_writable_hm),
                        writable_parent_instance: RwLock::new(HashMap::new()),
                        is_this_in_writable_path: true,
                        is_this_in_readable_path: false,
                    },
                };

                global_state.insert(new_writable_inode_val, new_content);
            }

            {
                let tmp1 = self.inode_to_string_mapping.try_read().unwrap();
                let name = tmp1.get(&new_writable_inode_val).unwrap();
                println!("calling clone fs for : {}", name);
            }

            {
                self.clone_fs(new_writable_inode_val, ses_id);
            }
        }
    }

    fn check_readable_path(
        &self,
        parent_inode_value: u64,
        name_to_be_searched: String,
    ) -> Option<u64> {
        let global_inode_to_content_mapping = self.inode_to_content_mapping.try_read().unwrap();
        if let Some(instance) = global_inode_to_content_mapping.get(&parent_inode_value) {
            let hm = instance
                .node_kind
                .children_of_directory()
                .unwrap()
                .try_read()
                .unwrap();
            if let Some(child) = hm.get(&name_to_be_searched) {
                println!("{} exists in the readable path", name_to_be_searched);
                Some(*child)
            } else {
                println!(
                    "{} does not exist in the readable path",
                    name_to_be_searched
                );
                None
            }
        } else {
            println!(
                "{} does not exist the readable path, in fact even the parent does not",
                name_to_be_searched
            );
            None
        }
    }

    fn check_writable_path(
        &self,
        parent_inode_value: u64,
        name_to_be_searched: String,
    ) -> Option<u64> {
        let global_inode_to_content_mapping = self.inode_to_content_mapping.try_read().unwrap();
        if let Some(instance) = global_inode_to_content_mapping.get(&parent_inode_value) {
            let hm = instance
                .node_kind
                .children_of_directory()
                .unwrap()
                .try_read()
                .unwrap();
            if let Some(child) = hm.get(&name_to_be_searched) {
                println!("{} exists in the wirtable path", name_to_be_searched);
                Some(*child)
            } else {
                println!(
                    "{} does not exist in the writable path",
                    name_to_be_searched
                );
                None
            }
        } else {
            println!(
                "{} does not exist the writable path, in fact even the parent does not",
                name_to_be_searched
            );
            None
        }
    }
}

fn increment_global_inode_val(mut val: RwLockWriteGuard<u64>) {
    *val += 1
}

fn increment_global_file_handle(mut val: RwLockWriteGuard<u64>) {
    *val += 1
}

fn strip(parent_path: PathBuf, child_path: PathBuf) -> String {
    let str_parent = parent_path.to_string_lossy().to_string();
    let str_child = child_path.to_string_lossy().to_string();

    let tmp = str_child.replace(&str_parent, "");
    tmp.replace("/", "")
}

impl Filesystem for UnionFs {
    fn lookup(
        &self,
        req: &fuser::Request,
        parent: INodeNo,
        name: &std::ffi::OsStr,
        reply: fuser::ReplyEntry,
    ) {
        let str_name = name.to_string_lossy().to_string();

        if str_name == "autorun.inf"
            || str_name == ".Trash"
            || str_name == ".Trash-1000"
            || str_name == ".xdg-volume-info"
        {
            reply.error(Errno::ENOENT);
            return;
        }

        let dur = Duration::default();

        let comm_pid = req.pid() as i32;
        println!("The request pid is : {}", comm_pid);
        if comm_pid == 13312 {
            let tmp = Process::new(comm_pid).unwrap();
            println!("The status of the process 13312 is  : {:?}", tmp.status());
        }
        let ses_id: i32;

        if let Ok(proc) = Process::new(comm_pid) {
            if let Ok(stat) = proc.stat() {
                ses_id = stat.tty_nr;
                println!("The session id is : {}", ses_id);

                let mut ses_maps = self.session_id_mapping.try_write().unwrap();

                if let Some(res) = ses_maps.get(&ses_id) {
                    println!(
                        "session_id exists with the root inode val for its respective root being : {}",
                        res
                    );
                } else if ses_id == 0 {
                    println!(
                        "session id of 0 gotten, skipping cuz it does nothing and is some background process from the gvfs"
                    );
                    return;
                } else {
                    println!(
                        "session id does not exist and we have to now add it to the hash map along with the root inode for that particular session id"
                    );
                    // have to instantiate session id
                    {
                        increment_global_inode_val(self.curr_inode_val.try_write().unwrap());
                    }
                    let cur = *self.curr_inode_val.try_read().unwrap();
                    ses_maps.insert(ses_id, cur);
                    let new_hm: HashMap<String, u64> = HashMap::new();

                    let content = InodeContent {
                        inode_attributes: make_attribute(cur, true),
                        node_kind: Node::Directory {
                            hash_of_children: RwLock::new(new_hm),
                            writable_parent_instance: RwLock::new(HashMap::new()),
                            is_this_in_readable_path: false,
                            is_this_in_writable_path: true,
                        },
                    };

                    {
                        let mut global_state = self.inode_to_content_mapping.try_write().unwrap();
                        global_state.insert(cur, content);
                        let mut inode_to_string_mapping =
                            self.inode_to_string_mapping.try_write().unwrap();
                        inode_to_string_mapping.insert(cur, "/".to_string());

                        let mut tmp = self.writable_to_readable_inode.try_write().unwrap();
                        tmp.insert(cur, Some(1));
                    }

                    {
                        println!(
                            "calling instantiate_session_id for session id : {} for the ",
                            ses_id
                        );
                        self.clone_fs(cur, ses_id);
                    }
                }

                // actual lookup logic
                let writable_root_instance = ses_maps.get(&ses_id).unwrap();

                if parent.0 == 1 {
                    let mut readable_inode: Option<u64> = None;
                    let mut writable_inode: Option<u64> = None;

                    if let Some(readable_result) =
                        self.check_readable_path(parent.0, str_name.clone())
                    {
                        readable_inode = Some(readable_result);
                    } else {
                        println!("Not found in readable path for parent root of inode val 1");
                    }

                    if let Some(writable_result) =
                        self.check_writable_path(*writable_root_instance, str_name.clone())
                    {
                        writable_inode = Some(writable_result);
                    } else {
                        println!("not found writable root inode path");
                    }

                    if writable_inode.is_some()
                        && readable_inode.is_some()
                        && writable_inode.unwrap() != 0
                        || writable_inode.is_some()
                            && readable_inode.is_none()
                            && writable_inode.unwrap() != 0
                    {
                        let global_state = self.inode_to_content_mapping.try_read().unwrap();
                        let attr = global_state.get(&writable_inode.unwrap()).unwrap();
                        reply.entry(&dur, &attr.inode_attributes, Generation(0));
                    } else if writable_inode.is_none() && readable_inode.is_some() {
                        println!(
                            "only readable path returned a non none value so checking if it returned a file for {}",
                            str_name
                        );
                        let global_state = self.inode_to_content_mapping.try_read().unwrap();
                        let attr = global_state.get(&readable_inode.unwrap()).unwrap();
                        /*
                        if attr.get_inode_kind() == FileType::RegularFile {
                            attr.node_kind
                                .update_writable_parent_instance(ses_id, );
                        }
                        */
                        reply.entry(&dur, &attr.inode_attributes, Generation(1));
                    } else {
                        println!("{} does not exist", str_name);
                        reply.error(Errno::ENOENT);
                    }
                } else {
                    let mut readable_inode: Option<u64> = None;
                    let mut writable_inode: Option<u64> = None;

                    println!("Dealing with parent of value : {}", parent.0);
                    let readable_mapping = self.writable_to_readable_inode.try_read().unwrap();
                    if let Some(relative_readable_inode) = readable_mapping.get(&parent.0) {
                        if let Some(readable_inode_val) = relative_readable_inode {
                            if let Some(readable_result) =
                                self.check_readable_path(*readable_inode_val, str_name.clone())
                            {
                                readable_inode = Some(readable_result);
                            } else {
                                println!(
                                    "[    lookup] : {} does not exist in teh readable path",
                                    str_name
                                );
                            }
                        } else {
                            println!(
                                "[lookup] : {} does not exist in teh readable path",
                                str_name
                            );
                        }
                    } else {
                        println!("[lookup] : parent in the readable path does not exist");
                    }

                    if let Some(writable_result) =
                        self.check_writable_path(parent.0, str_name.clone())
                    {
                        writable_inode = Some(writable_result);
                    } else {
                        println!("not found in writable path");
                    }

                    if readable_inode.is_some()
                        && writable_inode.is_some()
                        && writable_inode.unwrap() != 0
                        || readable_inode.is_none()
                            && writable_inode.is_some()
                            && writable_inode.unwrap() != 0
                    {
                        let global_instance = self.inode_to_content_mapping.try_read().unwrap();
                        let attr = global_instance.get(&writable_inode.unwrap()).unwrap();
                        reply.entry(&dur, &attr.inode_attributes, Generation(1));
                    } else if readable_inode.is_some() && writable_inode.is_none() {
                        println!(
                            "found {} only in readable path, so updated global state accordingly",
                            str_name.clone()
                        );
                        let global_instance = self.inode_to_content_mapping.try_read().unwrap();
                        let attr = global_instance.get(&readable_inode.unwrap()).unwrap();

                        if attr.get_inode_kind() == FileType::RegularFile {
                            attr.node_kind
                                .update_writable_parent_instance_for_files(ses_id, parent.0);
                        }
                        reply.entry(&dur, &attr.inode_attributes, Generation(1));
                    } else {
                        println!(
                            "Node with the name {} does not exist",
                            name.to_string_lossy()
                        );
                        reply.error(Errno::ENOENT);
                    }
                }
            } else {
                println!("was not able to get session id");
            }
        }
    }

    fn readdir(
        &self,
        req: &fuser::Request,
        mut writable_dir_inode: INodeNo,
        _fh: FileHandle,
        offset: u64,
        mut reply: fuser::ReplyDirectory,
    ) {
        let comm_pid = req.pid() as i32;
        println!("The request pid is : {}", comm_pid);
        if writable_dir_inode.0 == 1
            && let Ok(proc) = Process::new(comm_pid)
            && let Ok(stat) = proc.stat()
            && let Some(val) = self
                .session_id_mapping
                .try_read()
                .unwrap()
                .get(&stat.tty_nr)
        {
            writable_dir_inode.0 = *val;
        }

        println!("The writbale inode is {}", writable_dir_inode.0);

        let mut aggregate: Vec<(u64, FileType, String)> = Vec::new();
        let mut to_be_ommited: Vec<String> = Vec::new();

        let global_mapping = self.inode_to_content_mapping.try_read().unwrap();
        let writable_instance = global_mapping.get(&writable_dir_inode.0).unwrap();

        let writable_hm = writable_instance
            .node_kind
            .children_of_directory()
            .unwrap()
            .try_read()
            .unwrap();
        println!("The writable hm is : {:?}", writable_hm);
        let writable_iter = writable_hm.iter();

        for (name, inode_val) in writable_iter {
            if *inode_val == 0 {
                to_be_ommited.push(name.to_string());
                continue;
            }
            to_be_ommited.push(name.to_string());
            let file_type = global_mapping.get(inode_val).unwrap().get_inode_kind();
            if *inode_val != 0 {
                println!("inode val of : {}", *inode_val);
                aggregate.push((*inode_val, file_type, name.to_string()));
            }
        }

        println!("I am printing to be ommited : {:?}", to_be_ommited);
        if let Some(readable_dir_inode1) = self
            .writable_to_readable_inode
            .try_read()
            .unwrap()
            .get(&writable_dir_inode.0)
            && let Some(readable_dir_inode) = readable_dir_inode1
        {
            let readable_instance = global_mapping.get(readable_dir_inode).unwrap();
            let readable_hm = readable_instance
                .node_kind
                .children_of_directory()
                .unwrap()
                .try_read()
                .unwrap();
            let readable_iter = readable_hm.iter();

            for (name, inode_val) in readable_iter {
                if to_be_ommited.contains(name) {
                    println!("skipped : {}", name);
                    continue;
                }
                if let Some(final_type) = global_mapping.get(inode_val) {
                    let file_type = final_type.get_inode_kind();
                    aggregate.push((*inode_val, file_type, name.to_string()));
                } else {
                    println!("encountered deleted file : {}", name);
                }
            }
        }

        aggregate.push((writable_dir_inode.0, FileType::Directory, ".".to_string()));
        aggregate.push((writable_dir_inode.0, FileType::Directory, "..".to_string()));

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
        _uid: Option<u32>,
        _gid: Option<u32>,
        _size: Option<u64>,
        _atime: Option<fuser::TimeOrNow>,
        _mtime: Option<fuser::TimeOrNow>,
        _ctime: Option<SystemTime>,
        _fh: Option<fuser::FileHandle>,
        _crtime: Option<SystemTime>,
        _chgtime: Option<SystemTime>,
        _bkuptime: Option<SystemTime>,
        _flags: Option<fuser::BsdFileFlags>,
        reply: fuser::ReplyAttr,
    ) {
        println!("invoking setattr");
        let at = make_attribute(ino.0, false);
        //at.size = size.unwrap();
        //at.uid = uid.unwrap();
        //at.gid = gid.unwrap();
        let dur = Duration::default();
        reply.attr(&dur, &at);
    }

    fn write(
        &self,
        req: &fuser::Request,
        ino: INodeNo,
        _fh: fuser::FileHandle,
        offset: u64,
        data: &[u8],
        _write_flags: fuser::WriteFlags,
        _flags: fuser::OpenFlags,
        _lock_owner: Option<fuser::LockOwner>,
        reply: fuser::ReplyWrite,
    ) {
        let mut present_in_writable_path = None;
        let mut present_in_readable_path = None;
        println!("Enters the write function at offset of : {}", offset);

        {
            let inode_to_content_mapper = self.inode_to_content_mapping.try_read().unwrap();
            let tmp = inode_to_content_mapper.get(&ino.0).unwrap();
            let inode_instance_of_readable_node = &tmp.node_kind;
            present_in_writable_path = Some(
                inode_instance_of_readable_node
                    .is_it_present_in_writable_path()
                    .unwrap(),
            );

            present_in_readable_path = Some(
                inode_instance_of_readable_node
                    .is_it_present_in_readable_path()
                    .unwrap(),
            );
        }
        println!();

        if present_in_readable_path.unwrap() && !present_in_writable_path.unwrap() {
            increment_global_inode_val(self.curr_inode_val.try_write().unwrap());

            let mut inode_contentt: Option<InodeContent> = None;

            {
                let inode_to_content_mapper = self.inode_to_content_mapping.try_read().unwrap();
                let tmp = inode_to_content_mapper.get(&ino.0).unwrap();
                let inode_instance_of_readable_node = &tmp.node_kind;

                let readable_content = inode_instance_of_readable_node
                    .get_file_content()
                    .unwrap()
                    .try_read()
                    .unwrap();

                let attrs = make_attribute(*self.curr_inode_val.try_read().unwrap(), false);
                let bait_hm: RwLock<HashMap<i32, u64>> = RwLock::new(HashMap::new());
                let node = Node::File {
                    file_content: readable_content.clone().into(),
                    is_this_in_readable_path: false,
                    is_this_in_writable_path: true,
                    writable_parent_instance: bait_hm,
                };

                inode_contentt = Some(InodeContent {
                    inode_attributes: attrs,
                    node_kind: node,
                });
            }

            {
                let cur = *self.curr_inode_val.try_read().unwrap();
                let mut global_mapper = self.inode_to_content_mapping.try_write().unwrap();

                global_mapper.insert(cur, inode_contentt.unwrap());
                let mut tmp = self.inode_to_string_mapping.try_write().unwrap();
                let name = tmp.get(&ino.0).unwrap().clone();
                tmp.insert(cur, name.clone());
            }

            {
                let global_mapper = self.inode_to_content_mapping.try_read().unwrap();
                let tmp = global_mapper.get(&ino.0).unwrap();
                let inode_instance_of_readable_node = &tmp.node_kind;

                let comm_pid = req.pid() as i32;
                println!("The request pid is : {}", comm_pid);
                let ses_id: i32;

                if let Ok(proc) = Process::new(comm_pid)
                    && let Ok(stat) = proc.stat()
                {
                    ses_id = stat.tty_nr;
                    println!("The session id is : {}", ses_id);
                    let writable_parent_instance = inode_instance_of_readable_node
                        .get_writable_parent_instance_for_files(ses_id)
                        .unwrap();

                    let tmp = global_mapper.get(&writable_parent_instance).unwrap();

                    let inode_string_mapping = self.inode_to_string_mapping.try_read().unwrap();
                    let name = inode_string_mapping.get(&ino.0).unwrap().clone();

                    let mut children_hm = tmp
                        .node_kind
                        .children_of_directory()
                        .unwrap()
                        .try_write()
                        .unwrap();
                    children_hm.insert(name, *self.curr_inode_val.try_read().unwrap());

                    let node = global_mapper
                        .get(&self.curr_inode_val.try_read().unwrap())
                        .unwrap();

                    let readable_content = inode_instance_of_readable_node
                        .get_file_content()
                        .unwrap()
                        .try_read()
                        .unwrap();

                    let readable_iter = readable_content.iter();

                    let mut writable_instance_content = node
                        .node_kind
                        .get_file_content()
                        .unwrap()
                        .try_write()
                        .unwrap();
                    for vec_instance in readable_iter {
                        writable_instance_content.push(*vec_instance);
                    }
                }
            }

            {
                let cur = *self.curr_inode_val.try_read().unwrap();
                let mut global_mapper = self.inode_to_content_mapping.try_write().unwrap();
                let content = global_mapper.get_mut(&cur).unwrap();

                let mut node_instance = content
                    .node_kind
                    .get_file_content()
                    .unwrap()
                    .try_write()
                    .unwrap();

                let mut start = offset as usize;
                let initial_start_count = start;
                let data_to_be_inserted = data.iter();
                let mut written_amount = 0;
                for i in data_to_be_inserted {
                    start += 1;
                    node_instance.insert(start, *i);
                }
                written_amount = start - initial_start_count;
                reply.written(written_amount.try_into().unwrap());
            }
        } else if present_in_writable_path.unwrap() {
            {
                println!("comes into the writable path");
                let global_mapper = self.inode_to_content_mapping.try_read().unwrap();
                let content = global_mapper.get(&ino.0).unwrap();

                let mut node_instance = content
                    .node_kind
                    .get_file_content()
                    .unwrap()
                    .try_write()
                    .unwrap();

                let data_iter = data.iter();
                let mut counter = offset as usize;
                for individual_byte_to_be_inserted in data_iter {
                    println!(
                        "we are inserting {} at {} from the start",
                        *individual_byte_to_be_inserted, counter
                    );
                    node_instance.insert(counter, *individual_byte_to_be_inserted);
                    counter += 1;
                }
                reply.written(data.len() as u32);
            }
        }
    }

    fn open(
        &self,
        _req: &fuser::Request,
        ino: INodeNo,
        _flags: fuser::OpenFlags,
        reply: fuser::ReplyOpen,
    ) {
        println!(
            "enters the open function and the respective inode value is : {}",
            ino.0
        );

        {
            increment_global_file_handle(self.curr_file_handle.try_write().unwrap());
        }
        reply.opened(
            FileHandle(*self.curr_file_handle.try_read().unwrap()),
            FopenFlags::FOPEN_DIRECT_IO,
        );
    }

    fn read(
        &self,
        _req: &fuser::Request,
        ino: INodeNo,
        _fh: fuser::FileHandle,
        offset: u64,
        _size: u32,
        _flags: fuser::OpenFlags,
        _lock_owner: Option<fuser::LockOwner>,
        reply: fuser::ReplyData,
    ) {
        println!("The read function got invoked");
        let global_instance = self.inode_to_content_mapping.try_read().unwrap();
        let node_instance = &global_instance.get(&ino.0).unwrap().node_kind;
        let mut content = node_instance
            .get_file_content()
            .unwrap()
            .try_read()
            .unwrap()
            .clone();

        let buffer = content.split_off(offset as usize);

        reply.data(&buffer);
    }

    fn rename(
        &self,
        req: &fuser::Request,
        parent: INodeNo,
        name: &std::ffi::OsStr,
        newparent: INodeNo,
        newname: &std::ffi::OsStr,
        _flags: fuser::RenameFlags,
        reply: fuser::ReplyEmpty,
    ) {
        println!(
            "updating from {} to {} with parent changing from {} to {}",
            name.to_string_lossy(),
            newname.to_string_lossy(),
            parent.0,
            newparent.0
        );
        let mut need_to_update_global_state = false;
        let mut readable_content: Option<Vec<u8>> = None;
        let mut old_hm: Option<HashMap<String, u64>> = None;

        if parent.0 == 1
            && parent.0 == newparent.0
            && let Ok(proc) = Process::new(req.pid() as i32)
            && let Ok(stat) = proc.stat()
        {
            {
                let val: u64;
                let global_state = self.inode_to_content_mapping.try_read().unwrap();
                let session_id_mapping = self.session_id_mapping.try_read().unwrap();
                let writable_parent_inode = session_id_mapping.get(&stat.tty_nr).unwrap();
                let writable_instance = global_state.get(writable_parent_inode).unwrap();
                let mut writable_hm = writable_instance
                    .node_kind
                    .children_of_directory()
                    .unwrap()
                    .try_write()
                    .unwrap();
                if let Some(child_instance) = writable_hm.get(&name.to_string_lossy().to_string()) {
                    // child exists in teh wriatble path
                    val = *child_instance;
                    writable_hm.insert(name.to_string_lossy().to_string(), 0);
                    let mut inode_to_string_mappnig =
                        self.inode_to_string_mapping.try_write().unwrap();
                    inode_to_string_mappnig.remove_entry(&val);
                    inode_to_string_mappnig.insert(val, newname.to_string_lossy().to_string());
                } else {
                    // child does not exist in teh wriatble path
                    {
                        increment_global_inode_val(self.curr_inode_val.try_write().unwrap());
                    }
                    val = *self.curr_inode_val.try_read().unwrap();
                    need_to_update_global_state = true;
                    writable_hm.insert(name.to_string_lossy().to_string(), 0);
                    let mut inode_to_string_mappnig =
                        self.inode_to_string_mapping.try_write().unwrap();
                    inode_to_string_mappnig.insert(val, newname.to_string_lossy().to_string());
                }
                writable_hm.insert(newname.to_string_lossy().to_string(), val);
                let root_parent_content = global_state.get(&parent.0).unwrap();
                let root_hm = root_parent_content
                    .node_kind
                    .children_of_directory()
                    .unwrap()
                    .try_read()
                    .unwrap();
                let child = root_hm.get(&name.to_string_lossy().to_string()).unwrap();
                let child_instance = global_state.get(&child).unwrap();
                if child_instance.get_inode_kind() == FileType::RegularFile {
                    readable_content = Some(
                        child_instance
                            .node_kind
                            .get_file_content()
                            .unwrap()
                            .try_read()
                            .unwrap()
                            .to_vec(),
                    );
                    need_to_update_global_state = true;
                } else {
                    old_hm = Some(
                        child_instance
                            .node_kind
                            .children_of_directory()
                            .unwrap()
                            .try_read()
                            .unwrap()
                            .clone(),
                    );
                    need_to_update_global_state = true;
                }
            }
        } else if parent.0 == 1
            && let Ok(proc) = Process::new(req.pid() as i32)
            && let Ok(stat) = proc.stat()
        {
            let mut old_writable_child_inode: Option<u64> = None;
            {
                let global_mapping = self.inode_to_content_mapping.try_read().unwrap();
                let session_id_mapping = self.session_id_mapping.try_read().unwrap();
                let old_writable_parent_inode = session_id_mapping.get(&stat.tty_nr).unwrap();

                let old_writable_parent_instance =
                    global_mapping.get(old_writable_parent_inode).unwrap();
                let mut old_parent_writable_hm = old_writable_parent_instance
                    .node_kind
                    .children_of_directory()
                    .unwrap()
                    .try_write()
                    .unwrap();
                if let Some(child_inode) =
                    old_parent_writable_hm.get(&name.to_string_lossy().to_string())
                {
                    old_writable_child_inode = Some(*child_inode);
                    let child_instance = global_mapping.get(child_inode).unwrap();
                    if child_instance.get_inode_kind() == FileType::RegularFile {
                        readable_content = Some(
                            child_instance
                                .node_kind
                                .get_file_content()
                                .unwrap()
                                .try_read()
                                .unwrap()
                                .to_vec(),
                        );
                        need_to_update_global_state = true;
                    } else {
                        old_hm = Some(
                            child_instance
                                .node_kind
                                .children_of_directory()
                                .unwrap()
                                .try_read()
                                .unwrap()
                                .clone(),
                        );
                        need_to_update_global_state = true;
                    }
                    old_parent_writable_hm.insert(name.to_string_lossy().to_string(), 0);
                    increment_global_inode_val(self.curr_inode_val.try_write().unwrap());
                } else {
                    let old_parent = global_mapping.get(&parent.0).unwrap();
                    let old_parent_hm = old_parent
                        .node_kind
                        .children_of_directory()
                        .unwrap()
                        .try_read()
                        .unwrap();
                    let child_inode = old_parent_hm
                        .get(&name.to_string_lossy().to_string())
                        .unwrap();
                    let child_instance = global_mapping.get(child_inode).unwrap();
                    if child_instance.get_inode_kind() == FileType::RegularFile {
                        readable_content = Some(
                            child_instance
                                .node_kind
                                .get_file_content()
                                .unwrap()
                                .try_read()
                                .unwrap()
                                .to_vec(),
                        );
                    } else {
                        old_hm = Some(
                            child_instance
                                .node_kind
                                .children_of_directory()
                                .unwrap()
                                .try_read()
                                .unwrap()
                                .clone(),
                        );
                    }
                    old_parent_writable_hm.insert(name.to_string_lossy().to_string(), 0);
                }
            }

            if old_writable_child_inode.is_some() {
                let mut global_mapping = self.inode_to_content_mapping.try_write().unwrap();
                global_mapping.remove_entry(&old_writable_child_inode.unwrap());

                let mut inode_string_mapping = self.inode_to_string_mapping.try_write().unwrap();
                inode_string_mapping.remove_entry(&old_writable_child_inode.unwrap());
            }

            {
                let global_mapping = self.inode_to_content_mapping.try_read().unwrap();
                let new_parent = global_mapping.get(&newparent.0).unwrap();
                let mut new_hm = new_parent
                    .node_kind
                    .children_of_directory()
                    .unwrap()
                    .try_write()
                    .unwrap();
                if let Some(child) = new_hm.get(&newname.to_string_lossy().to_string()) {
                    // child with the name already exists hence must return error
                    if *child != 0 {
                        println!("child with the name already exists in the new parent")
                    } else {
                        {
                            increment_global_inode_val(self.curr_inode_val.try_write().unwrap());
                        }
                        new_hm.insert(
                            newname.to_string_lossy().to_string(),
                            *self.curr_inode_val.try_read().unwrap(),
                        );
                        need_to_update_global_state = true;
                    }
                } else {
                    {
                        increment_global_inode_val(self.curr_inode_val.try_write().unwrap());
                    }
                    new_hm.insert(
                        newname.to_string_lossy().to_string(),
                        *self.curr_inode_val.try_read().unwrap(),
                    );
                    need_to_update_global_state = true;
                }
            }
        } else if parent.0 != 0 && parent.0 != newparent.0 {
            let mut old_child_inode: Option<u64> = None;
            {
                let global_mapping = self.inode_to_content_mapping.try_read().unwrap();
                let initial_parent_instance = global_mapping.get(&parent.0).unwrap();
                let mut initial_hm = initial_parent_instance
                    .node_kind
                    .children_of_directory()
                    .unwrap()
                    .try_write()
                    .unwrap();

                if let Some(val) = initial_hm.get(&name.to_string_lossy().to_string()) {
                    old_child_inode = Some(*val);
                    let child_instance = global_mapping.get(val).unwrap();
                    if child_instance.get_inode_kind() == FileType::RegularFile {
                        readable_content = Some(
                            child_instance
                                .node_kind
                                .get_file_content()
                                .unwrap()
                                .try_read()
                                .unwrap()
                                .to_vec(),
                        );
                        initial_hm.insert(name.to_string_lossy().to_string(), 0);
                    } else {
                        old_hm = Some(
                            child_instance
                                .node_kind
                                .children_of_directory()
                                .unwrap()
                                .try_read()
                                .unwrap()
                                .clone(),
                        );
                        initial_hm.insert(name.to_string_lossy().to_string(), 0);
                    }
                } else {
                    need_to_update_global_state = false;
                }
            }

            if need_to_update_global_state {
                let global_mapping = self.inode_to_content_mapping.try_read().unwrap();
                let new_parent_instance = global_mapping.get(&newparent.0).unwrap();
                let new_hm = new_parent_instance
                    .node_kind
                    .children_of_directory()
                    .unwrap()
                    .try_read()
                    .unwrap();
                need_to_update_global_state =
                    new_hm.get(&newname.to_string_lossy().to_string()).is_none()
            }

            if need_to_update_global_state {
                {
                    increment_global_inode_val(self.curr_inode_val.try_write().unwrap());
                }
                let global_mapping = self.inode_to_content_mapping.try_read().unwrap();
                let new_parent_instance = global_mapping.get(&newparent.0).unwrap();
                let mut new_hm = new_parent_instance
                    .node_kind
                    .children_of_directory()
                    .unwrap()
                    .try_write()
                    .unwrap();
                new_hm.insert(
                    newname.to_string_lossy().to_string(),
                    *self.curr_inode_val.try_read().unwrap(),
                );
            }

            if need_to_update_global_state {
                let mut global_mapping = self.inode_to_content_mapping.try_write().unwrap();
                global_mapping.remove_entry(&old_child_inode.unwrap());

                let mut inode_string_mapping = self.inode_to_string_mapping.try_write().unwrap();
                inode_string_mapping.remove_entry(&old_child_inode.unwrap());
            }
        }

        if need_to_update_global_state {
            {
                let mut global_state = self.inode_to_content_mapping.try_write().unwrap();
                if old_hm.is_some() {
                    let new_inode_content = InodeContent {
                        inode_attributes: make_attribute(
                            *self.curr_inode_val.try_read().unwrap(),
                            true,
                        ),
                        node_kind: Node::Directory {
                            hash_of_children: RwLock::new(old_hm.unwrap()),
                            writable_parent_instance: RwLock::new(HashMap::new()),
                            is_this_in_readable_path: false,
                            is_this_in_writable_path: true,
                        },
                    };
                    global_state
                        .insert(*self.curr_inode_val.try_read().unwrap(), new_inode_content);
                } else if readable_content.is_some() {
                    let new_inode_content = InodeContent {
                        inode_attributes: make_attribute(
                            *self.curr_inode_val.try_read().unwrap(),
                            false,
                        ),
                        node_kind: Node::File {
                            file_content: RwLock::new(readable_content.unwrap()),
                            is_this_in_readable_path: false,
                            is_this_in_writable_path: true,
                            writable_parent_instance: RwLock::new(HashMap::new()),
                        },
                    };
                    global_state
                        .insert(*self.curr_inode_val.try_read().unwrap(), new_inode_content);
                }
            }
            reply.ok();
        } else {
            reply.error(Errno::ENOENT);
        }
    }

    fn rmdir(
        &self,
        req: &fuser::Request,
        parent: INodeNo,
        name: &std::ffi::OsStr,
        reply: fuser::ReplyEmpty,
    ) {
        println!(
            "rmdir got invoked for : name {} , parent : {}",
            name.to_string_lossy(),
            parent.0
        );

        if parent.0 == 1 {
            println!("does it enter here");
            let readable_child_inode: u64;

            {
                let gobal_mapping = self.inode_to_content_mapping.try_read().unwrap();
                let readable_parent_instance = gobal_mapping.get(&parent.0).unwrap();
                let parent_hm = readable_parent_instance
                    .node_kind
                    .children_of_directory()
                    .unwrap()
                    .try_read()
                    .unwrap();
                println!("The parent hash map is : {:?}", parent_hm);
                readable_child_inode = *parent_hm.get(&name.to_string_lossy().to_string()).unwrap();
            }

            if let Ok(proc) = Process::new(req.pid() as i32)
                && let Ok(stat) = proc.stat()
            {
                println!("The session id gotten here is : {}", stat.tty_nr);
                let writable_child_instance: u64;
                /*
                {
                    let gobal_mapping = self.inode_to_content_mapping.try_read().unwrap();
                    let readable_child_instance = gobal_mapping.get(&readable_child_inode).unwrap();
                    let writable_parent_inode = readable_child_instance
                        .node_kind
                        .get_writable_parent_instance_for_dirs(stat.tty_nr)
                        .unwrap();
                    let writable_instance = gobal_mapping.get(&writable_parent_inode).unwrap();
                    let mut writable_hm = writable_instance
                        .node_kind
                        .children_of_directory()
                        .unwrap()
                        .try_write()
                        .unwrap();
                    writable_child_instance = *writable_hm
                        .get(&name.to_string_lossy().to_string())
                        .unwrap();
                    writable_hm.insert(name.to_string_lossy().to_string(), 0);
                }
                */

                {
                    let session_id_mapper = self.session_id_mapping.try_read().unwrap();
                    let writable_root_inode = *session_id_mapper.get(&stat.tty_nr).unwrap();

                    let global_mapper = self.inode_to_content_mapping.try_read().unwrap();
                    let writable_instance = global_mapper.get(&writable_root_inode).unwrap();
                    let mut writable_hm = writable_instance
                        .node_kind
                        .children_of_directory()
                        .unwrap()
                        .try_write()
                        .unwrap();
                    writable_child_instance = *writable_hm
                        .get(&name.to_string_lossy().to_string())
                        .unwrap();
                    writable_hm.insert(name.to_string_lossy().to_string(), 0);
                }
                {
                    let mut gobal_mapping = self.inode_to_content_mapping.try_write().unwrap();
                    gobal_mapping
                        .remove_entry(&writable_child_instance)
                        .unwrap();

                    let mut inode_to_string_mapping =
                        self.inode_to_string_mapping.try_write().unwrap();
                    inode_to_string_mapping
                        .remove_entry(&writable_child_instance)
                        .unwrap();
                }
            }
        } else {
            println!("does it enter here");
            let child_inode_val: u64;
            {
                let global_mapping = self.inode_to_content_mapping.try_read().unwrap();
                let parent_instance = global_mapping.get(&parent.0).unwrap();
                let mut parent_hm = parent_instance
                    .node_kind
                    .children_of_directory()
                    .unwrap()
                    .try_write()
                    .unwrap();
                println!("The parent hash map is : {:?}", parent_hm);
                child_inode_val = *parent_hm.get(&name.to_string_lossy().to_string()).unwrap();
                parent_hm.insert(name.to_string_lossy().to_string(), 0);
            }
            {
                let mut gobal_mapping = self.inode_to_content_mapping.try_write().unwrap();
                gobal_mapping.remove_entry(&child_inode_val).unwrap();

                let mut inode_to_string_mapping = self.inode_to_string_mapping.try_write().unwrap();
                inode_to_string_mapping
                    .remove_entry(&child_inode_val)
                    .unwrap();
            }
        }
        reply.ok();
    }

    fn unlink(
        &self,
        req: &fuser::Request,
        parent: INodeNo,
        name: &std::ffi::OsStr,
        reply: fuser::ReplyEmpty,
    ) {
        println!(
            "unlink got called for parents inode value of : {} and child name : {}",
            parent.0,
            name.to_string_lossy().to_string()
        );
        let deleted_inode_val: u64;
        let readable_path_presence: bool;
        let writable_path_presence: bool;
        {
            let global_mapping = self.inode_to_content_mapping.try_read().unwrap();
            let parent_instance = global_mapping.get(&parent.0).unwrap();

            let parent_hm = parent_instance
                .node_kind
                .children_of_directory()
                .unwrap()
                .try_read()
                .unwrap();
            println!(
                "The inode val : {} has the following state : {:?}",
                parent.0, parent_hm
            );
            let tmp = parent_hm.get(&name.to_string_lossy().to_string());

            if tmp.is_some() {
                // this branch means that the file exists in the writable hm
                deleted_inode_val = *tmp.unwrap();
            } else {
                // this means it does not exist in teh writable hm
                println!("Does it enter here");
                let writable_to_readable_mapping =
                    self.writable_to_readable_inode.try_read().unwrap();
                deleted_inode_val = writable_to_readable_mapping
                    .get(&parent.0)
                    .unwrap()
                    .unwrap();
                // next step is to fin
            }

            let child_instance = global_mapping.get(&deleted_inode_val).unwrap();
            if let Some(tmp) = child_instance.node_kind.is_it_present_in_writable_path()
                && tmp
            {
                writable_path_presence = true;
            } else {
                // writable path returned None
                writable_path_presence = false;
            }

            if let Some(tmp) = child_instance.node_kind.is_it_present_in_readable_path()
                && tmp
            {
                readable_path_presence = true;
            } else {
                readable_path_presence = false;
            }
            println!("writable path : {:?}", writable_path_presence);
        }

        if readable_path_presence && writable_path_presence
            || writable_path_presence && !readable_path_presence
        {
            {
                println!("Deleting the node : {}", name.to_string_lossy().clone());
                let global_mapping = self.inode_to_content_mapping.try_read().unwrap();
                let parent_instance = global_mapping.get(&parent.0).unwrap();

                let mut parent_hm = parent_instance
                    .node_kind
                    .children_of_directory()
                    .unwrap()
                    .try_write()
                    .unwrap();

                parent_hm.insert(name.to_string_lossy().to_string(), 0);
            }

            {
                let mut global_mapping = self.inode_to_content_mapping.try_write().unwrap();
                global_mapping.remove_entry(&deleted_inode_val);

                let mut inode_to_string_mapping = self.inode_to_string_mapping.try_write().unwrap();
                inode_to_string_mapping.remove_entry(&deleted_inode_val);
            }
        } else if parent.0 == 1 {
            let global_mapping = self.inode_to_content_mapping.try_read().unwrap();
            //let tmp = global_mapping.get(&deleted_inode_val).unwrap();
            let comm_pid = req.pid() as i32;

            if let Ok(proc) = Process::new(comm_pid)
                && let Ok(stat) = proc.stat()
            {
                let ses_id = stat.tty_nr;
                println!("The session id is : {}", ses_id);
                let readable_parent_hm = global_mapping
                    .get(&parent.0)
                    .unwrap()
                    .node_kind
                    .children_of_directory()
                    .unwrap();
                println!(
                    "The readable parent hm is : {:?} and we are searching for name : {}",
                    readable_parent_hm,
                    name.to_string_lossy()
                );
                let tmp = readable_parent_hm.try_read().unwrap();
                let readable_file_inode = tmp.get(&name.to_string_lossy().to_string()).unwrap();
                let readable_file_instance = global_mapping.get(readable_file_inode).unwrap();
                let writable_parent = readable_file_instance
                    .node_kind
                    .get_writable_parent_instance_for_files(ses_id)
                    .unwrap();
                println!(
                    "The wriatble parnet into which we are inserting is {}",
                    writable_parent
                );
                let writable_parent_instance = global_mapping.get(&writable_parent).unwrap();
                let mut writable_hm = writable_parent_instance
                    .node_kind
                    .children_of_directory()
                    .unwrap()
                    .try_write()
                    .unwrap();
                writable_hm.insert(name.to_string_lossy().to_string(), 0);
            }
        } else {
            println!(
                "we have gotten readable path here, where the actual file in the readable part is : {}",
                deleted_inode_val
            );

            let global_mapping = self.inode_to_content_mapping.try_read().unwrap();
            let tmp = global_mapping.get(&deleted_inode_val).unwrap();
            let comm_pid = req.pid() as i32;

            if let Ok(proc) = Process::new(comm_pid)
                && let Ok(stat) = proc.stat()
            {
                let ses_id = stat.tty_nr;
                println!("The session id is : {}", ses_id);
                let readable_parent_hm = global_mapping
                    .get(&deleted_inode_val)
                    .unwrap()
                    .node_kind
                    .children_of_directory()
                    .unwrap();
                println!(
                    "The readable parent hm is : {:?} and we are searching for name : {}",
                    readable_parent_hm,
                    name.to_string_lossy()
                );
                let tmp = readable_parent_hm.try_read().unwrap();
                let readable_file_inode = tmp.get(&name.to_string_lossy().to_string()).unwrap();
                let readable_file_instance = global_mapping.get(readable_file_inode).unwrap();
                let writable_parent = readable_file_instance
                    .node_kind
                    .get_writable_parent_instance_for_files(ses_id)
                    .unwrap();
                println!(
                    "The wriatble parnet into which we are inserting is {}",
                    writable_parent
                );
                let writable_parent_instance = global_mapping.get(&writable_parent).unwrap();
                let mut writable_hm = writable_parent_instance
                    .node_kind
                    .children_of_directory()
                    .unwrap()
                    .try_write()
                    .unwrap();
                writable_hm.insert(name.to_string_lossy().to_string(), 0);
            }
        }
        reply.ok();
    }

    //TODO : need to handle the case where there exists a node with the same name already
    fn mknod(
        &self,
        _req: &fuser::Request,
        parent: INodeNo,
        name: &std::ffi::OsStr,
        _mode: u32,
        _umask: u32,
        _rdev: u32,
        reply: fuser::ReplyEntry,
    ) {
        println!("calling mknod for {}", name.to_string_lossy());
        {
            increment_global_inode_val(self.curr_inode_val.try_write().unwrap());
        }
        let new_inode_val = *self.curr_inode_val.try_read().unwrap();

        {
            let parent_instance = self.inode_to_content_mapping.try_read().unwrap();
            let mut parent_hm = parent_instance
                .get(&parent.0)
                .unwrap()
                .node_kind
                .children_of_directory()
                .unwrap()
                .try_write()
                .unwrap();

            parent_hm.insert(name.to_string_lossy().to_string(), new_inode_val);
        }

        let mut global_indode_mapping = self.inode_to_content_mapping.try_write().unwrap();
        let new_vec: RwLock<Vec<u8>> = RwLock::new(Vec::new());
        let new_attrs = make_attribute(new_inode_val, false);
        let bait_hm: RwLock<HashMap<i32, u64>> = RwLock::new(HashMap::new());
        let new_inodecontent = InodeContent {
            inode_attributes: new_attrs,
            node_kind: Node::File {
                file_content: new_vec,
                is_this_in_readable_path: false,
                is_this_in_writable_path: true,
                writable_parent_instance: bait_hm,
            },
        };
        global_indode_mapping.insert(new_inode_val, new_inodecontent);

        let mut writable_to_readable_instance =
            self.writable_to_readable_inode.try_write().unwrap();
        writable_to_readable_instance.insert(new_inode_val, None);
        let dur = Duration::default();
        reply.entry(&dur, &new_attrs, Generation(1));
    }

    fn mkdir(
        &self,
        req: &fuser::Request,
        parent: INodeNo,
        name: &std::ffi::OsStr,
        _mode: u32,
        _umask: u32,
        reply: fuser::ReplyEntry,
    ) {
        let mut final_status = Some(false);
        {
            increment_global_inode_val(self.curr_inode_val.try_write().unwrap());
        }
        let new_inode_val = *self.curr_inode_val.try_read().unwrap();

        let comm_pid = req.pid() as i32;
        if parent.0 == 1
            && let Ok(proc) = Process::new(comm_pid)
            && let Ok(stat) = proc.stat()
        {
            let ses_id = stat.tty_nr;
            println!("The session id is : {}", ses_id);

            let session_id_mapper = self.session_id_mapping.try_read().unwrap();
            let writable_root_inode = session_id_mapper.get(&ses_id).unwrap();
            let global_mapper = self.inode_to_content_mapping.try_read().unwrap();
            let writable_instance = global_mapper.get(writable_root_inode).unwrap();
            let mut writable_children = writable_instance
                .node_kind
                .children_of_directory()
                .unwrap()
                .try_write()
                .unwrap();

            let root_instance = global_mapper.get(&parent.0).unwrap();
            let root_hm = root_instance
                .node_kind
                .children_of_directory()
                .unwrap()
                .try_read()
                .unwrap();

            if writable_children
                .get(&name.to_string_lossy().to_string())
                .is_none()
                && root_hm.get(&name.to_string_lossy().to_string()).is_none()
            {
                writable_children.insert(name.to_string_lossy().to_string(), new_inode_val);
                println!(
                    "Inserted directory with name {} and inode value : {} which has parent {}",
                    name.to_string_lossy(),
                    new_inode_val,
                    parent.0
                );
                final_status = Some(true);
            }
        } else {
            let parent_instance = self.inode_to_content_mapping.try_read().unwrap();
            let mut parent_hm = parent_instance
                .get(&parent.0)
                .unwrap()
                .node_kind
                .children_of_directory()
                .unwrap()
                .try_write()
                .unwrap();

            if parent_hm.get(&name.to_string_lossy().to_string()).is_none() {
                parent_hm.insert(name.to_string_lossy().to_string(), new_inode_val);
                println!(
                    "Inserted directory with name {} and inode value : {} which has parent {}",
                    name.to_string_lossy(),
                    new_inode_val,
                    parent.0
                );
                final_status = Some(true);
            }
        }

        if final_status.unwrap() {
            let mut global_indode_mapping = self.inode_to_content_mapping.try_write().unwrap();
            let new_attrs = make_attribute(new_inode_val, true);
            let bait_hm: RwLock<HashMap<String, u64>> = RwLock::new(HashMap::new());
            let new_inodecontent = InodeContent {
                inode_attributes: new_attrs,
                node_kind: Node::Directory {
                    hash_of_children: bait_hm,
                    writable_parent_instance: RwLock::new(HashMap::new()),
                    is_this_in_readable_path: false,
                    is_this_in_writable_path: true,
                },
            };
            global_indode_mapping.insert(new_inode_val, new_inodecontent);

            let mut writable_to_readable_instance =
                self.writable_to_readable_inode.try_write().unwrap();
            writable_to_readable_instance.insert(new_inode_val, None);
            let dur = Duration::default();
            reply.entry(&dur, &new_attrs, Generation(1));
        } else {
            reply.error(Errno::EEXIST);
        }
    }

    fn create(
        &self,
        req: &fuser::Request,
        parent: INodeNo,
        name: &std::ffi::OsStr,
        _mode: u32,
        _umask: u32,
        _flags: i32,
        reply: fuser::ReplyCreate,
    ) {
        println!("calling create for {}", name.to_string_lossy());
        let mut final_status: Option<bool> = Some(false);
        let comm_pid = req.pid() as i32;
        if parent.0 == 1
            && let Ok(proc) = Process::new(comm_pid)
            && let Ok(stat) = proc.stat()
        {
            let ses_id = stat.tty_nr;
            println!("The session id is : {}", ses_id);

            let session_id_mapper = self.session_id_mapping.try_read().unwrap();
            let writable_root_inode = session_id_mapper.get(&ses_id).unwrap();
            let global_mapper = self.inode_to_content_mapping.try_read().unwrap();
            let writable_instance = global_mapper.get(writable_root_inode).unwrap();
            let mut writable_children = writable_instance
                .node_kind
                .children_of_directory()
                .unwrap()
                .try_write()
                .unwrap();

            let root_instance = global_mapper.get(&parent.0).unwrap();
            let root_hm = root_instance
                .node_kind
                .children_of_directory()
                .unwrap()
                .try_read()
                .unwrap();

            let writable_status = writable_children.get(&name.to_string_lossy().to_string());
            let readable_status = root_hm.get(&name.to_string_lossy().to_string());
            if readable_status.is_some() && writable_status.is_some() {
                {
                    increment_global_inode_val(self.curr_inode_val.try_write().unwrap());
                }
                let new_inode_val = *self.curr_inode_val.try_read().unwrap();

                writable_children.insert(name.to_string_lossy().to_string(), new_inode_val);
                final_status = Some(true);
            }
        } else {
            let parent_instance = self.inode_to_content_mapping.try_read().unwrap();
            let mut parent_hm = parent_instance
                .get(&parent.0)
                .unwrap()
                .node_kind
                .children_of_directory()
                .unwrap()
                .try_write()
                .unwrap();

            if parent_hm.get(&name.to_string_lossy().to_string()).is_none() {
                {
                    increment_global_inode_val(self.curr_inode_val.try_write().unwrap());
                }
                let new_inode_val = *self.curr_inode_val.try_read().unwrap();
                parent_hm.insert(name.to_string_lossy().to_string(), new_inode_val);
                println!(
                    "Inserted file with name {} and inode value : {} which has parent {}",
                    name.to_string_lossy(),
                    new_inode_val,
                    parent.0
                );
                final_status = Some(true);
            }
        }

        if final_status.unwrap() {
            let new_inode_val = *self.curr_inode_val.try_read().unwrap();
            let mut global_indode_mapping = self.inode_to_content_mapping.try_write().unwrap();
            let new_vec: RwLock<Vec<u8>> = RwLock::new(Vec::new());
            let new_attrs = make_attribute(new_inode_val, false);
            let bait_hm: RwLock<HashMap<i32, u64>> = RwLock::new(HashMap::new());
            let new_inodecontent = InodeContent {
                inode_attributes: new_attrs,
                node_kind: Node::File {
                    file_content: new_vec,
                    is_this_in_readable_path: false,
                    is_this_in_writable_path: true,
                    writable_parent_instance: bait_hm,
                },
            };
            global_indode_mapping.insert(new_inode_val, new_inodecontent);

            let mut writable_to_readable_instance =
                self.writable_to_readable_inode.try_write().unwrap();
            writable_to_readable_instance.insert(new_inode_val, None);
            let dur = Duration::default();
            reply.created(
                &dur,
                &new_attrs,
                Generation(0),
                FileHandle(*self.curr_file_handle.try_read().unwrap()),
                FopenFlags::FOPEN_DIRECT_IO,
            );
        } else {
            reply.error(Errno::EEXIST);
        }
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

fn instantiate_fs(file_system_instance: &UnionFs, path: &PathBuf, parent_inode_value: u64) {
    let parent_val = parent_inode_value;

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
                    let next_val = file_system_instance.curr_inode_val.try_write().unwrap();
                    increment_global_inode_val(next_val);
                }

                {
                    let mut global_state = file_system_instance
                        .inode_to_content_mapping
                        .try_write()
                        .unwrap();

                    let mut inode_to_string_mapping = file_system_instance
                        .inode_to_string_mapping
                        .try_write()
                        .unwrap();
                    inode_to_string_mapping.insert(
                        *file_system_instance.curr_inode_val.read().unwrap(),
                        str_child_path.clone(),
                    );
                    println!(
                        "inserted name for : {} as {}",
                        *file_system_instance.curr_inode_val.read().unwrap(),
                        str_child_path.clone()
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

                if pathh.is_file() {
                    println!("file detected");
                    let file_attr =
                        make_attribute(*file_system_instance.curr_inode_val.read().unwrap(), false);

                    println!(
                        "The file attribute in the readable path is : {:?}",
                        file_attr
                    );

                    let actual_content = fs::read(&pathh).unwrap();
                    let tmp_hm: HashMap<i32, u64> = HashMap::new();
                    let writable_parent_hm: RwLock<HashMap<i32, u64>> = RwLock::new(tmp_hm);

                    let new_node = InodeContent {
                        inode_attributes: file_attr,
                        node_kind: Node::File {
                            file_content: RwLock::new(actual_content),
                            is_this_in_writable_path: false,
                            is_this_in_readable_path: true,
                            writable_parent_instance: writable_parent_hm,
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
                    println!("Done with the file");
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
                            writable_parent_instance: RwLock::new(HashMap::new()),
                            is_this_in_readable_path: true,
                            is_this_in_writable_path: false,
                        },
                    };

                    {
                        let second_global_state =
                            file_system_instance.inode_to_content_mapping.try_write();
                        match second_global_state {
                            Ok(mut t) => {
                                println!("ok now it inserted");
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

                    println!(
                        "the inode value being inserted is : {}",
                        file_system_instance.curr_inode_val.read().unwrap().clone()
                    );

                    let tmp = *file_system_instance.curr_inode_val.read().unwrap();
                    println!("The parent inode value being called next is : {}", tmp);
                    println!();
                    instantiate_fs(file_system_instance, &pathh.clone(), tmp);
                }
            }
        }
        Err(er) => {
            println!("could not read from the directory cuz : {}", er);
        }
    }
    println!("returning from instantiate_fs");
}

fn main() {
    let cmdline_args: Vec<String> = env::args().collect();
    let pathToBeMounted = &cmdline_args[2];
    println!("The path to be mounted is : {}", pathToBeMounted);
    let pathname: PathBuf = PathBuf::from(pathToBeMounted);

    let mut fileSystem_instance = UnionFs::new(pathname.clone());

    fileSystem_instance.primary_pathname = RwLock::new(pathname.clone());

    if pathname.is_dir() {
        instantiate_fs(&fileSystem_instance, &pathname, 1);
    }

    let mut cfg = Config::default();
    let v = vec![MountOption::RW, MountOption::AutoUnmount];
    cfg.mount_options = v;
    cfg.acl = fuser::SessionACL::All;
    cfg.n_threads = Some(4);
    cfg.clone_fd = false;

    fuser::mount2(fileSystem_instance, pathname.clone(), &cfg).unwrap();
}
