//! Standard filesystem paths must round-trip as strings without lossy replacement.

use facet::Facet;
use facet_json::{from_str, to_string, to_string_pretty};
use facet_reflect::Peek;
use std::borrow::Cow;
use std::collections::BTreeMap;
use std::path::PathBuf;

#[derive(Debug, Facet, PartialEq)]
struct PathSettings {
    work_dirs: Vec<PathBuf>,
    open_dirs: BTreeMap<PathBuf, bool>,
}

#[test]
fn paths_in_nested_collections_preserve_their_text() {
    let paths = [
        "",
        "workspace with spaces/雪",
        r"C:\builds\a\b",
        r"\\server\share\📁",
    ];
    let settings = PathSettings {
        work_dirs: paths.iter().map(PathBuf::from).collect(),
        open_dirs: paths
            .iter()
            .map(|path| (PathBuf::from(path), true))
            .collect(),
    };

    for json in [
        to_string(&settings).unwrap(),
        to_string_pretty(&settings).unwrap(),
    ] {
        assert_eq!(from_str::<PathSettings>(&json).unwrap(), settings);
        let representation: serde_json::Value = serde_json::from_str(&json).unwrap();
        for (index, path) in paths.iter().enumerate() {
            assert_eq!(representation["work_dirs"][index].as_str(), Some(*path));
            assert_eq!(representation["open_dirs"][*path].as_bool(), Some(true));
        }
    }
}

#[test]
fn borrowed_path_buffers_use_their_utf8_text() {
    let path = PathBuf::from(r"C:\workspace with spaces\雪");
    let json = to_string(&&path).unwrap();
    assert_eq!(from_str::<String>(&json).unwrap(), path.to_str().unwrap());
}

#[test]
fn the_path_proxy_borrows_the_existing_utf8_text() {
    let path = PathBuf::from(r"C:\workspace with spaces\雪");
    let proxy = Peek::new(&path)
        .custom_serialization_from_shape()
        .unwrap()
        .unwrap();

    proxy.with_peek(|value| {
        let Cow::Borrowed(text) = value.get::<Cow<'_, str>>().unwrap() else {
            panic!("serializing a UTF-8 PathBuf must borrow its existing text");
        };
        assert_eq!(*text, path.to_str().unwrap());
        assert_eq!(text.as_ptr(), path.to_str().unwrap().as_ptr());
    });
}

#[cfg(windows)]
fn non_utf8_path() -> PathBuf {
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt;

    PathBuf::from(OsString::from_wide(&[b'a' as u16, 0xd800]))
}

#[cfg(unix)]
fn non_utf8_path() -> PathBuf {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;

    PathBuf::from(OsString::from_vec(vec![b'a', 0xff]))
}

#[cfg(any(windows, unix))]
#[test]
fn non_utf8_paths_are_rejected_at_the_nested_value_or_key() {
    let settings = PathSettings {
        work_dirs: vec![non_utf8_path()],
        open_dirs: BTreeMap::new(),
    };
    let error = to_string(&settings).unwrap_err().to_string();
    assert!(error.contains("path contains non-UTF-8 data"), "{error}");
    assert!(error.contains(".work_dirs[0]"), "{error}");

    let settings = PathSettings {
        work_dirs: Vec::new(),
        open_dirs: BTreeMap::from([(non_utf8_path(), true)]),
    };
    let error = to_string(&settings).unwrap_err().to_string();
    assert!(error.contains("path contains non-UTF-8 data"), "{error}");
    assert!(error.contains(".open_dirs"), "{error}");

    let path = non_utf8_path();
    let error = to_string(&&path).unwrap_err().to_string();
    assert!(error.contains("path contains non-UTF-8 data"), "{error}");
}
