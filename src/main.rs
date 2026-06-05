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

fn make_dir_attribute(inode_val: u64) -> FileAttr {
    let now = SystemTime::now();
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
}

// have to peroform a .is_dir() on this before calling this
fn make_base_writable_path(
    parent_inode_value: u64,
    base_pathname: PathBuf,
    pathname: PathBuf,
    mut list: Vec<PathBuf>,
    uf: &mut unionFS,
) -> Vec<PathBuf> {
    println!("The base pathname is : {:?}", &base_pathname);
    println!("The generic pathname is : {:?}", &pathname);
    let foo2 = fs::read_dir(&pathname);
    match foo2 {
        Ok(r) => {
            for bar in r {
                let name_cropped = &bar
                    .unwrap()
                    .path()
                    .to_path_buf()
                    .strip_prefix(&base_pathname)
                    .unwrap()
                    .to_path_buf();
                println!("name : {}", name_cropped.to_string_lossy());

                if base_pathname.is_dir() {
                    list.push(name_cropped.clone());
                    // here I need to add this directory to the parents hash_of_children and also
                    // assign it an inode value : which keeps track of the string name to inode
                    // mapping, that function will then return the inode number that was assigned
                    // to this directory
                    // subsequently this inode value is then passed into the next
                    // make_base_writable_path, which will go on doing this
                    //
                    // I also need to make a separate function which does the same for the files

                    let new_inode_val = uf.fill_up_hashes(
                        name_cropped.to_string_lossy().to_string(),
                        parent_inode_value,
                    );

                    println!("does not come here");

                    let mut new_hm: HashMap<String, u64> = HashMap::new();
                    let tmp = make_dir_attribute(new_inode_val);
                    let tmp2: InodeContent = InodeContent {
                        inode_attributes: tmp,
                        node_kind: Node::Directory {
                            hash_of_children: new_hm,
                        },
                    };
                    uf.mapping.insert(new_inode_val, tmp2);

                    list = make_base_writable_path(
                        new_inode_val,
                        base_pathname.clone(),
                        name_cropped.to_path_buf(),
                        list,
                        uf,
                    );
                }
            }
        }
        Err(_) => println!("check if the path is an actual directory"),
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
        list_of_directories = make_base_writable_path(
            1,
            pathname.clone(),
            pathname.clone(),
            list_of_directories,
            &mut fileSystem_instance,
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
