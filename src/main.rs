use fuser::{Config, Errno, FileAttr, FileType, Filesystem, Generation, INodeNo, MountOption};
use std::{
    collections::HashMap,
    env,
    fs::{self},
    path::PathBuf,
    time::{Duration, SystemTime},
};

#[derive(Debug)]
enum Node {
    Directory {
        hash_of_children: HashMap<String, u64>,
    },
    File {
        file_content: String,
    },
}

impl Node {
    fn just_access_children_without_editing_anything(&self) -> Option<HashMap<String, u64>> {
        match self {
            Self::Directory { hash_of_children } => Some(hash_of_children.clone()),
            Self::File { .. } => None,
        }
    }

    fn actual_access_to_children(&mut self) -> Option<&mut HashMap<String, u64>> {
        match self {
            Self::Directory { hash_of_children } => Some(hash_of_children),
            Self::File { .. } => None,
        }
    }
}

#[derive(Debug)]
struct InodeContent {
    inode_attributes: FileAttr,
    node_kind: Node,
}

impl InodeContent {
    fn InodeKind(&self) -> FileType {
        match &self.node_kind {
            Node::Directory { .. } => FileType::Directory,
            Node::File { .. } => FileType::RegularFile,
        }
    }
}

struct unionFS {
    next_inode_value: u64,
    _next_user_number: u16,
    mapping: HashMap<u64, InodeContent>,
}

impl unionFS {
    fn new() -> Self {
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
                hash_of_children: HashMap::new(),
            },
        };

        let mut node_mapping = HashMap::new();
        node_mapping.insert("/".to_string(), 1);

        let mut additional_mapping_test = HashMap::new();
        additional_mapping_test.insert(1, root);
        unionFS {
            mapping: additional_mapping_test,
            next_inode_value: 2,
            _next_user_number: 2,
        }
    }

    fn fill_up_hashes(&mut self, name: String, parent_inode_value: u64) -> u64 {
        println!("does fill_up_hashes even get invoked ? ");
        let new_inode_value = self.next_inode_value;
        self.next_inode_value += 1;

        self.mapping
            .get_mut(&parent_inode_value)
            .unwrap()
            .node_kind
            .actual_access_to_children()
            .unwrap()
            .entry(name)
            .or_insert(new_inode_value);

        new_inode_value
    }
}

impl Filesystem for unionFS {
    fn lookup(
        &self,
        _req: &fuser::Request,
        parent: INodeNo,
        name: &std::ffi::OsStr,
        reply: fuser::ReplyEntry,
    ) {
        // perform lookup
        println!("inside the lookup function");
        let name_local_format = name.to_string_lossy().to_string();
        println!(
            "The name entered into the lookup function is : {} and parent inode value is : {}",
            name_local_format, parent.0
        );

        let fooo = self.mapping.iter().clone();
        for i in fooo {
            println!("{:?}", i);
            println!();
            println!();
        }

        let possible_inode_content = self.mapping.get(&parent.0);
        match possible_inode_content {
            Some(inode_instance) => {
                println!(
                    "So the corresponding inode value for {} exists in {} hashmap",
                    name_local_format, parent.0
                );
                let returned_children = inode_instance
                    .node_kind
                    .just_access_children_without_editing_anything()
                    .unwrap();
                let res = returned_children.get(&name_local_format);
                match res {
                    Some(childs_inode_value) => {
                        println!("Does it even enter here");
                        let res2 = self.mapping.get(childs_inode_value).unwrap();
                        let f_attr = res2.inode_attributes;
                        let d = Duration::from_nanos(20);
                        reply.entry(&d, &f_attr, Generation(0))
                    }
                    None => {
                        println!(
                            "it returned NONE on the inode value, so the InodeContent exists but the inode value does not ? if that makes sense"
                        );
                        reply.error(Errno::ENOENT)
                    }
                }
            }
            None => reply.error(fuser::Errno::ENOENT),
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
        println!("performing readdir function");
        println!(
            "The inode number on which we are performing the readdir is : {}",
            ino.0
        );

        let temp = &mut self
            .mapping
            .get(&ino.0)
            .unwrap()
            .node_kind
            .just_access_children_without_editing_anything()
            .unwrap();

        let mut aggregate: Vec<(u64, FileType, String)> = Vec::new();

        for (i, ii) in temp {
            let ft = &self.mapping.get(ii).unwrap().InodeKind();
            aggregate.push((*ii, *ft, i.to_string()));
        }
        aggregate.push((ino.0, FileType::Directory, ".".to_string()));
        aggregate.push((ino.0, FileType::Directory, "..".to_string()));

        for (i, (inode, file_type, name)) in aggregate.into_iter().enumerate().skip(offset as usize)
        {
            if reply.add(INodeNo(inode), (i + 1) as u64, file_type, &name) {
                break;
            }
        }
    }

    fn getattr(
        &self,
        _req: &fuser::Request,
        ino: INodeNo,
        _fh: Option<fuser::FileHandle>,
        reply: fuser::ReplyAttr,
    ) {
        let d = Duration::default();
        match self.mapping.get(&ino.0) {
            Some(i) => reply.attr(&d, &i.inode_attributes),
            None => reply.error(Errno::ENOENT),
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

fn instantiate_fs(
    uf: &mut unionFS,
    primary_pathname: PathBuf,
    //_secondary_pathname: PathBuf,
    mut list: Vec<PathBuf>,
    parent_inode_value: u64,
) -> Vec<PathBuf> {
    let the_dir_instance = fs::read_dir(&primary_pathname);
    match the_dir_instance {
        Ok(r) => {
            for i in r {
                let next_inode_val = uf.next_inode_value;
                uf.next_inode_value += 1;

                if let Some(parent_inode_instance) = uf.mapping.get_mut(&parent_inode_value) {
                    if let Some(parent_mapping) =
                        parent_inode_instance.node_kind.actual_access_to_children()
                    {
                        parent_mapping.insert(
                            i.as_ref().unwrap().file_name().into_string().unwrap(),
                            next_inode_val,
                        );
                        list.push(i.as_ref().unwrap().file_name().into());

                        let type_of_file = i.as_ref().unwrap().metadata().unwrap().is_dir();
                        if type_of_file {
                            let hm_instance: HashMap<String, u64> = HashMap::new();
                            let inode_instance = InodeContent {
                                inode_attributes: make_attribute(next_inode_val, true),
                                node_kind: Node::Directory {
                                    hash_of_children: hm_instance,
                                },
                            };
                            uf.mapping.insert(next_inode_val, inode_instance);

                            list = instantiate_fs(uf, i.unwrap().path(), list, next_inode_val);
                        } else {
                            println!(
                                "its a file and its path is {}",
                                i.as_ref().unwrap().path().to_string_lossy()
                            );
                            let tmp = i.unwrap().path();
                            let file_contents = fs::read_to_string(tmp).unwrap();
                            let inode_instance = InodeContent {
                                inode_attributes: make_attribute(next_inode_val, false),
                                node_kind: Node::File {
                                    file_content: file_contents,
                                },
                            };
                            uf.mapping.insert(next_inode_val, inode_instance);
                        }
                    } else {
                        println!("parent hashmap does not exist");
                    }
                } else {
                    println!("no such parent inode number : {}", parent_inode_value);
                }
            }
        }
        Err(e) => println!("something went wrong and gave the following error : {}", e),
    }
    list
}

fn main() {
    let mut fileSystem_instance = unionFS::new();

    let cmdline_args: Vec<String> = env::args().collect();
    let pathToBeMounted = &cmdline_args[2];
    println!("The path to be mounted is : {}", pathToBeMounted);
    let pathname: PathBuf = PathBuf::from(pathToBeMounted);

    let mut list_of_directories: Vec<PathBuf> = Vec::new();
    if pathname.is_dir() {
        list_of_directories = instantiate_fs(
            &mut fileSystem_instance,
            pathToBeMounted.into(),
            list_of_directories,
            1,
        );
    }

    let testt = list_of_directories.iter();
    for i in testt {
        println!("tet {}", i.to_string_lossy());
    }

    let v = vec![MountOption::RO, MountOption::AutoUnmount];
    let mut cfg = Config::default();
    cfg.mount_options = v;
    cfg.acl = fuser::SessionACL::All;
    cfg.n_threads = Some(1);
    cfg.clone_fd = false;

    fuser::mount2(fileSystem_instance, pathname.clone(), &cfg).unwrap();
}
