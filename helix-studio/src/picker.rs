use std::error::Error;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use helix_term::filter_picker_entry;
use helix_term::ui::{get_excluded_types, Picker, PickerColumn};
use helix_view::{theme::Style, Editor};
use tui::text::{Span, Spans};

pub struct FilePickerData {
    root: PathBuf,
    directory_style: Style,
}

pub type FilePicker = Picker<PathBuf, FilePickerData>;

pub fn file_picker(editor: &Editor, root: PathBuf) -> FilePicker {
    use ignore::WalkBuilder;

    let config = editor.config();
    let data = FilePickerData {
        root: root.clone(),
        directory_style: editor.theme.get("ui.text.directory"),
    };

    let dedup_symlinks = config.file_picker.deduplicate_links;
    let absolute_root = root.canonicalize().unwrap_or_else(|_| root.clone());

    let mut walk_builder = WalkBuilder::new(&root);

    let mut files = walk_builder
        .hidden(config.file_picker.hidden)
        .parents(config.file_picker.parents)
        .ignore(config.file_picker.ignore)
        .follow_links(config.file_picker.follow_symlinks)
        .git_ignore(config.file_picker.git_ignore)
        .git_global(config.file_picker.git_global)
        .git_exclude(config.file_picker.git_exclude)
        .sort_by_file_name(|name1, name2| name1.cmp(name2))
        .max_depth(config.file_picker.max_depth)
        .filter_entry(move |entry| filter_picker_entry(entry, &absolute_root, dedup_symlinks))
        .add_custom_ignore_filename(helix_loader::config_dir().join("ignore"))
        .add_custom_ignore_filename(".helix/ignore")
        .types(get_excluded_types())
        .build()
        .filter_map(|entry| {
            let entry = entry.ok()?;
            if !entry.path().is_file() {
                return None;
            }
            Some(entry.into_path())
        });

    let columns = [
        PickerColumn::new("name", |item: &PathBuf, _: &FilePickerData| {
            let name = item
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default();
            Spans::from(vec![Span::raw(name)]).into()
        }),
        PickerColumn::new("path", |item: &PathBuf, data: &FilePickerData| {
            let path = item.strip_prefix(&data.root).unwrap_or(item);
            let dir = path
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
                .map(|parent| parent.to_string_lossy().into_owned())
                .unwrap_or_default();
            Spans::from(vec![Span::styled(dir, data.directory_style)]).into()
        }),
    ];

    let picker = Picker::new(columns, 0, [], data, move |cx, path: &PathBuf, action| {
        if let Err(e) = cx.editor.open(path, action) {
            let err = if let Some(err) = e.source() {
                format!("{}", err)
            } else {
                format!("unable to open \"{}\"", path.display())
            };
            cx.editor.set_error(err);
        }
    })
    .with_preview(|_editor, path| Some((path.as_path().into(), None)));

    let injector = picker.injector();
    let timeout = Instant::now() + Duration::from_millis(30);

    let current = current_path(editor);
    if let Some(current) = current.clone() {
        if injector.push(current).is_err() {
            return picker;
        }
    }

    let mut hit_timeout = false;
    for file in &mut files {
        if Some(&file) == current.as_ref() {
            continue;
        }
        if injector.push(file).is_err() {
            break;
        }
        if Instant::now() >= timeout {
            hit_timeout = true;
            break;
        }
    }
    if hit_timeout {
        std::thread::spawn(move || {
            for file in files {
                if Some(&file) == current.as_ref() {
                    continue;
                }
                if injector.push(file).is_err() {
                    break;
                }
            }
        });
    }

    picker
}

fn current_path(editor: &Editor) -> Option<PathBuf> {
    let view = editor.tree.try_get(editor.tree.focus)?;
    editor.document(view.doc)?.path().map(Path::to_path_buf)
}
