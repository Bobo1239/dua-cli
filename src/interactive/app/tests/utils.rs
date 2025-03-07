use anyhow::{Context, Error, Result};
use dua::{
    traverse::{EntryData, Tree, TreeIndex},
    ByteFormat, TraversalSorting, WalkOptions,
};
use itertools::Itertools;
use jwalk::{DirEntry, WalkDir};
use petgraph::prelude::NodeIndex;
use std::{
    env::temp_dir,
    ffi::OsStr,
    fmt,
    fs::{copy, create_dir_all, remove_dir, remove_file},
    io::ErrorKind,
    path::{Path, PathBuf},
};
use tui::backend::TestBackend;
use tui_react::Terminal;

use crate::interactive::{app::tests::FIXTURE_PATH, Interaction, TerminalApp};

pub fn into_keys<'a>(
    bytes: impl Iterator<Item = &'a u8> + 'a,
) -> impl Iterator<Item = crosstermion::input::Key> + 'a {
    bytes.map(|b| crosstermion::input::Key::Char(std::char::from_u32(*b as u32).unwrap()))
}

pub fn node_by_index(app: &TerminalApp, id: TreeIndex) -> &EntryData {
    app.traversal.tree.node_weight(id).unwrap()
}

pub fn node_by_name(app: &TerminalApp, name: impl AsRef<OsStr>) -> &EntryData {
    node_by_index(app, index_by_name(app, name))
}

pub fn index_by_name_and_size(
    app: &TerminalApp,
    name: impl AsRef<OsStr>,
    size: Option<u128>,
) -> TreeIndex {
    let name = name.as_ref();
    let t: Vec<_> = app
        .traversal
        .tree
        .node_indices()
        .map(|idx| (idx, node_by_index(app, idx)))
        .filter_map(|(idx, e)| {
            if e.name == name && size.map(|s| s == e.size).unwrap_or(true) {
                Some(idx)
            } else {
                None
            }
        })
        .collect();
    match t.len() {
        1 => t[0],
        0 => panic!("Node named '{}' not found in tree", name.to_string_lossy()),
        n => panic!("Node named '{}' found {} times", name.to_string_lossy(), n),
    }
}

pub fn index_by_name(app: &TerminalApp, name: impl AsRef<OsStr>) -> TreeIndex {
    index_by_name_and_size(app, name, None)
}

pub struct WritableFixture {
    pub root: PathBuf,
}

impl Drop for WritableFixture {
    fn drop(&mut self) {
        delete_recursive(&self.root).ok();
    }
}

fn delete_recursive(path: impl AsRef<Path>) -> Result<()> {
    let mut files: Vec<_> = Vec::new();
    let mut dirs: Vec<_> = Vec::new();

    for entry in WalkDir::new(&path)
        .parallelism(jwalk::Parallelism::Serial)
        .into_iter()
    {
        let entry: DirEntry<_> = entry?;
        let p = entry.path();
        match p.is_dir() {
            true => dirs.push(p),
            false => files.push(p),
        }
    }

    files
        .iter()
        .map(|f| remove_file(f).map_err(Error::from))
        .chain(
            dirs.iter()
                .sorted_by_key(|p| p.components().count())
                .rev()
                .map(|d| {
                    remove_dir(d)
                        .with_context(|| format!("Could not delete '{}'", d.display()))
                        .map_err(Error::from)
                }),
        )
        .collect::<Result<_, _>>()
}

fn copy_recursive(src: impl AsRef<Path>, dst: impl AsRef<Path>) -> Result<(), Error> {
    for entry in WalkDir::new(&src)
        .parallelism(jwalk::Parallelism::Serial)
        .into_iter()
    {
        let entry: DirEntry<_> = entry?;
        let entry_path = entry.path();
        entry_path
            .strip_prefix(&src)
            .map_err(Error::from)
            .and_then(|relative_entry_path| {
                let dst = dst.as_ref().join(relative_entry_path);
                if entry_path.is_dir() {
                    create_dir_all(dst).map_err(Into::into)
                } else {
                    copy(&entry_path, dst)
                        .map(|_| ())
                        .or_else(|e| match e.kind() {
                            ErrorKind::AlreadyExists => Ok(()),
                            _ => Err(e),
                        })
                        .map_err(Into::into)
                }
            })?;
    }
    Ok(())
}

impl From<&'static str> for WritableFixture {
    fn from(fixture_name: &str) -> Self {
        const TEMP_TLD_DIRNAME: &str = "dua-unit";

        let src = fixture(fixture_name);
        let dst = temp_dir().join(TEMP_TLD_DIRNAME);
        create_dir_all(&dst).unwrap();

        let dst = dst.join(fixture_name);
        copy_recursive(src, &dst).unwrap();
        WritableFixture { root: dst }
    }
}

impl AsRef<Path> for WritableFixture {
    fn as_ref(&self) -> &Path {
        &self.root
    }
}

pub fn fixture(p: impl AsRef<Path>) -> PathBuf {
    Path::new(FIXTURE_PATH).join(p)
}

pub fn fixture_str(p: impl AsRef<Path>) -> String {
    fixture(p).to_str().unwrap().to_owned()
}

pub fn initialized_app_and_terminal_with_closure(
    fixture_paths: &[impl AsRef<Path>],
    mut convert: impl FnMut(&Path) -> PathBuf,
) -> Result<(Terminal<TestBackend>, TerminalApp), Error> {
    let mut terminal = new_test_terminal()?;
    std::env::set_current_dir(Path::new(env!("CARGO_MANIFEST_DIR")))?;

    let input_paths = fixture_paths.iter().map(|c| convert(c.as_ref())).collect();
    let app = TerminalApp::initialize(
        &mut terminal,
        WalkOptions {
            threads: 1,
            byte_format: ByteFormat::Metric,
            apparent_size: true,
            count_hard_links: false,
            sorting: TraversalSorting::AlphabeticalByFileName,
            cross_filesystems: false,
        },
        input_paths,
        Interaction::None,
    )?
    .map(|(_, app)| app);
    Ok((
        terminal,
        app.expect("app that didn't try to abort iteration"),
    ))
}

pub fn new_test_terminal() -> std::io::Result<Terminal<TestBackend>> {
    Terminal::new(TestBackend::new(40, 20))
}

pub fn initialized_app_and_terminal_from_paths(
    fixture_paths: &[PathBuf],
) -> Result<(Terminal<TestBackend>, TerminalApp), Error> {
    fn to_path_buf(p: &Path) -> PathBuf {
        p.to_path_buf()
    }
    initialized_app_and_terminal_with_closure(fixture_paths, to_path_buf)
}

pub fn initialized_app_and_terminal_from_fixture(
    fixture_paths: &[&str],
) -> Result<(Terminal<TestBackend>, TerminalApp), Error> {
    #[allow(clippy::redundant_closure)]
    // doesn't actually work that way due to borrowchk - probably a bug
    initialized_app_and_terminal_with_closure(fixture_paths, |p| fixture(p))
}

pub fn sample_01_tree() -> Tree {
    let mut tree = Tree::new();
    {
        let mut add_node = make_add_node(&mut tree);
        #[cfg(not(windows))]
        let root_size = 1259070;
        #[cfg(windows)]
        let root_size = 1259069;
        let rn = add_node("", root_size, None);
        {
            let sn = add_node(&fixture_str("sample-01"), root_size, Some(rn));
            {
                add_node(".hidden.666", 666, Some(sn));
                add_node("a", 256, Some(sn));
                add_node("b.empty", 0, Some(sn));
                #[cfg(not(windows))]
                add_node("c.lnk", 1, Some(sn));
                #[cfg(windows)]
                add_node("c.lnk", 0, Some(sn));
                let dn = add_node("dir", 1258024, Some(sn));
                {
                    add_node("1000bytes", 1000, Some(dn));
                    add_node("dir-a.1mb", 1_000_000, Some(dn));
                    add_node("dir-a.kb", 1024, Some(dn));
                    let en = add_node("empty-dir", 0, Some(dn));
                    {
                        add_node(".gitkeep", 0, Some(en));
                    }
                    let sub = add_node("sub", 256_000, Some(dn));
                    {
                        add_node("dir-sub-a.256kb", 256_000, Some(sub));
                    }
                }
                add_node("z123.b", 123, Some(sn));
            }
        }
    }
    tree
}

pub fn sample_02_tree() -> Tree {
    let mut tree = Tree::new();
    {
        let mut add_node = make_add_node(&mut tree);
        let root_size = 1540;
        let rn = add_node("", root_size, None);
        {
            let sn = add_node(
                Path::new(FIXTURE_PATH).join("sample-02").to_str().unwrap(),
                root_size,
                Some(rn),
            );
            {
                add_node("a", 256, Some(sn));
                add_node("b", 1, Some(sn));
                let dn = add_node("dir", 1283, Some(sn));
                {
                    add_node("c", 257, Some(dn));
                    add_node("d", 2, Some(dn));
                    let en = add_node("empty-dir", 0, Some(dn));
                    {
                        add_node(".gitkeep", 0, Some(en));
                    }
                    let sub = add_node("sub", 1024, Some(dn));
                    {
                        add_node("e", 1024, Some(sub));
                    }
                }
            }
        }
    }
    tree
}

pub fn make_add_node(t: &mut Tree) -> impl FnMut(&str, u128, Option<NodeIndex>) -> NodeIndex + '_ {
    move |name, size, maybe_from_idx| {
        let n = t.add_node(EntryData {
            name: PathBuf::from(name),
            size,
            metadata_io_error: false,
        });
        if let Some(from) = maybe_from_idx {
            t.add_edge(from, n, ());
        }
        n
    }
}

pub fn debug(item: impl fmt::Debug) -> String {
    format!("{:?}", item)
}
