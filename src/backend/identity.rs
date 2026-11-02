//! Who owns what.
//!
//! Every path in this server is built from user-supplied identity: a username
//! becomes a directory, an album name becomes a directory, a photo path names
//! a file. These types are the only way to turn such input into something the
//! filesystem layer will accept. Constructing one validates; after that the
//! value cannot be a path escape, so downstream code does not re-check what it
//! was already given.
//!
//! Registration is stricter than `Owner::new` on purpose: the owner type only
//! guarantees what a path component and a JSON key need, so accounts created
//! before the registration rules were tightened still load.
use std::fmt;
use std::ops::Deref;
use std::path::{Path, PathBuf};

/// A username that is safe as a single path component.
#[derive(
    Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
#[serde(transparent)]
pub(crate) struct Owner(String);

impl Owner {
    pub(crate) fn new(raw: &str) -> Option<Self> {
        let name = raw.trim();
        if name.is_empty() || name.len() > 255 {
            return None;
        }
        if name == "." || name == ".." {
            return None;
        }
        if name.contains('/') || name.contains('\\') || name.contains('\0') {
            return None;
        }
        Some(Self(name.to_string()))
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }

    /// This owner's data root (`dist/data/{owner}`).
    pub(crate) fn root(&self, data_path: &Path) -> PathBuf {
        data_path.join(&self.0)
    }

    /// `<owner>/albums`.
    pub(crate) fn albums_root(&self, data_path: &Path) -> PathBuf {
        self.root(data_path).join("albums")
    }
}

impl PartialEq<str> for Owner {
    fn eq(&self, other: &str) -> bool {
        self.0 == other
    }
}

impl PartialEq<&str> for Owner {
    fn eq(&self, other: &&str) -> bool {
        self.0 == *other
    }
}

impl PartialEq<Owner> for str {
    fn eq(&self, other: &Owner) -> bool {
        self == other.0
    }
}

impl std::borrow::Borrow<str> for Owner {
    fn borrow(&self) -> &str {
        &self.0
    }
}

impl Deref for Owner {
    type Target = str;

    fn deref(&self) -> &str {
        &self.0
    }
}

impl AsRef<str> for Owner {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl AsRef<Path> for Owner {
    fn as_ref(&self) -> &Path {
        Path::new(&self.0)
    }
}

impl fmt::Display for Owner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// An album directory name that cannot escape `albums/`.
///
/// This is the *current* name, which can change on rename. Anything that must
/// survive a rename belongs in `AlbumMeta.cache`, not here.
#[derive(
    Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
#[serde(transparent)]
pub(crate) struct AlbumName(String);

impl AlbumName {
    pub(crate) fn new(raw: &str) -> Option<Self> {
        let name = raw.trim();
        if name.is_empty() {
            return None;
        }
        if name.contains('/') || name.contains('\\') || name.contains('\0') {
            return None;
        }
        if name.starts_with('.') || name.starts_with("__") {
            return None;
        }
        Some(Self(name.to_string()))
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

impl PartialEq<str> for AlbumName {
    fn eq(&self, other: &str) -> bool {
        self.0 == other
    }
}

impl PartialEq<&str> for AlbumName {
    fn eq(&self, other: &&str) -> bool {
        self.0 == *other
    }
}

impl std::borrow::Borrow<str> for AlbumName {
    fn borrow(&self) -> &str {
        &self.0
    }
}

impl Deref for AlbumName {
    type Target = str;

    fn deref(&self) -> &str {
        &self.0
    }
}

impl AsRef<str> for AlbumName {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl AsRef<Path> for AlbumName {
    fn as_ref(&self) -> &Path {
        Path::new(&self.0)
    }
}

impl fmt::Display for AlbumName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Which album, for a viewer who may be a sharee rather than the owner.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct AlbumRef {
    owner: Owner,
    name: AlbumName,
}

impl AlbumRef {
    pub(crate) fn new(owner: Owner, name: AlbumName) -> Self {
        Self { owner, name }
    }

    /// Parse an album identifier as the API accepts it: `name` for the caller's
    /// own album, `owner/name` for a shared one. Only the first `/` separates.
    pub(crate) fn parse(input: &str, caller: &Owner) -> Option<Self> {
        if let Some((owner, name)) = input.split_once('/') {
            let owner = Owner::new(owner)?;
            let name = AlbumName::new(name)?;
            return Some(Self { owner, name });
        }
        Some(Self {
            owner: caller.clone(),
            name: AlbumName::new(input)?,
        })
    }

    pub(crate) fn owner(&self) -> &Owner {
        &self.owner
    }

    pub(crate) fn name(&self) -> &AlbumName {
        &self.name
    }

    /// `<data>/{owner}/albums/{name}`.
    pub(crate) fn dir(&self, data_path: &Path) -> PathBuf {
        self.owner.albums_root(data_path).join(self.name.as_str())
    }

    /// Identifier in the API's `owner/name` form.
    pub(crate) fn api_key(&self) -> String {
        format!("{}/{}", self.owner, self.name)
    }
}

impl fmt::Display for AlbumRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.owner, self.name)
    }
}

/// A photo path relative to one album directory.
///
/// Segments are validated, `\` becomes `/`, and the value is never absolute.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct PhotoRel(String);

impl PhotoRel {
    pub(crate) fn new(raw: &str) -> Option<Self> {
        let text = raw.trim().replace('\\', "/");
        if text.is_empty() || text.len() > 1024 || text.as_bytes().len() > u16::MAX as usize {
            return None;
        }
        if text.starts_with('/') || text.contains('\0') {
            return None;
        }
        // `C:/...` is a Windows drive path, not a relative one.
        let first = text.split('/').next().unwrap_or("");
        if first.len() == 2 && first.ends_with(':') {
            return None;
        }
        for part in text.split('/') {
            if part.is_empty() || part == "." || part == ".." {
                return None;
            }
        }
        Some(Self(text))
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

impl Deref for PhotoRel {
    type Target = str;

    fn deref(&self) -> &str {
        &self.0
    }
}

impl AsRef<str> for PhotoRel {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl AsRef<Path> for PhotoRel {
    fn as_ref(&self) -> &Path {
        Path::new(&self.0)
    }
}

impl fmt::Display for PhotoRel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// One photo: the album it belongs to plus its path inside that album.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct PhotoRef {
    album: AlbumRef,
    rel: PhotoRel,
}

impl PhotoRef {
    pub(crate) fn new(album: AlbumRef, rel: PhotoRel) -> Self {
        Self { album, rel }
    }

    pub(crate) fn album(&self) -> &AlbumRef {
        &self.album
    }

    pub(crate) fn rel(&self) -> &PhotoRel {
        &self.rel
    }

    /// Path as the photo API names it: `albums/{album_name}/{rel}`.
    pub(crate) fn api_path(&self) -> String {
        format!("albums/{}/{}", self.album.name, self.rel)
    }

    /// Cache-key component: the API path, which is relative to the data root.
    pub(crate) fn cache_rel(&self) -> String {
        self.api_path()
    }
}

#[cfg(test)]
mod tests {
    use super::{AlbumName, AlbumRef, Owner, PhotoRef, PhotoRel};
    use std::path::Path;

    #[test]
    fn owner_rejects_path_components_that_escape() {
        for bad in ["", "  ", ".", "..", "a/b", "a\\b", "a\0b"] {
            assert!(Owner::new(bad).is_none(), "accepted {bad:?}");
        }
        let owner = Owner::new("daniel").expect("valid owner");
        assert_eq!(owner.as_str(), "daniel");
        assert_eq!(owner.root(Path::new("/data")), Path::new("/data/daniel"));
        assert_eq!(
            owner.albums_root(Path::new("/data")),
            Path::new("/data/daniel/albums")
        );
        // Registration is stricter; a legacy account still loads.
        assert!(Owner::new("old-user").is_some());
    }

    #[test]
    fn album_name_matches_the_sanitizer() {
        for bad in ["", "  ", "a/b", "a\\b", "a\0b", ".hidden", "__uploading"] {
            assert!(AlbumName::new(bad).is_none(), "accepted {bad:?}");
        }
        assert_eq!(AlbumName::new(" 测试 ").unwrap().as_str(), "测试");
    }

    #[test]
    fn album_ref_parses_own_and_shared_identifiers() {
        let caller = Owner::new("daniel").unwrap();
        let own = AlbumRef::parse("trip", &caller).unwrap();
        assert_eq!(own.owner().as_str(), "daniel");
        assert_eq!(own.name().as_str(), "trip");
        assert_eq!(own.api_key(), "daniel/trip");

        let shared = AlbumRef::parse("irene/trip", &caller).unwrap();
        assert_eq!(shared.owner().as_str(), "irene");
        assert_eq!(shared.name().as_str(), "trip");
        // An album name is one path component, so a second slash is not a name.
        assert!(AlbumRef::parse("irene/trip/2026", &caller).is_none());
    }

    #[test]
    fn own_album_identifier_never_splits_on_a_slash_in_the_name() {
        let caller = Owner::new("daniel").unwrap();
        // Album names cannot contain '/', so this parses as owner/name and the
        // name must remain a single component.
        assert!(AlbumRef::parse("a/b", &caller).is_some());
        assert!(AlbumRef::parse("a/", &caller).is_none());
        assert!(AlbumRef::parse("/b", &caller).is_none());
        assert!(AlbumRef::parse("../x", &caller).is_none());
    }

    #[test]
    fn photo_rel_never_leaves_its_album() {
        for bad in [
            "",
            "  ",
            "/etc/passwd",
            "../bob/a.jpg",
            "a/../../b",
            "a//b",
            "a/./b",
            "C:/a.jpg",
        ] {
            assert!(PhotoRel::new(bad).is_none(), "accepted {bad:?}");
        }
        // A colon inside a name is a legal Unix filename character.
        assert!(PhotoRel::new("2026:01/a.jpg").is_some());
        // Surrounding whitespace is not part of the name; Windows separators
        // become the separators the rest of the server uses.
        assert_eq!(
            PhotoRel::new("  nest/b.JPG  ").unwrap().as_str(),
            "nest/b.JPG"
        );
        assert_eq!(PhotoRel::new("a\\b\\c.jpg").unwrap().as_str(), "a/b/c.jpg");
        assert_eq!(
            PhotoRel::new("scenes\\000000000139.jpg").unwrap().as_str(),
            "scenes/000000000139.jpg"
        );
        assert_eq!(
            PhotoRel::new("2026/测试.jpg").unwrap().as_str(),
            "2026/测试.jpg"
        );
    }

    #[test]
    fn photo_ref_builds_the_api_path() {
        let caller = Owner::new("daniel").unwrap();
        let album = AlbumRef::parse("trip", &caller).unwrap();
        let photo = PhotoRef::new(album, PhotoRel::new("2026/a.jpg").unwrap());
        assert_eq!(photo.api_path(), "albums/trip/2026/a.jpg");
        assert_eq!(photo.cache_rel(), photo.api_path());
        assert_eq!(
            photo.album().dir(Path::new("/data")),
            Path::new("/data/daniel/albums/trip")
        );
    }
}
