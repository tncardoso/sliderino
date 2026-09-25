//! Assets embedded in the binary: the Lucide icons the editor uses and the
//! Inter font faces.

use std::borrow::Cow;

use gpui_kit::assets::{Assets, icon_assets};
use gpui_kit::{AssetSource, Result, SharedString};

icon_assets!(
    EditorIcons,
    [
        MousePointer2,
        Hand,
        Type,
        Square,
        Circle,
        Slash,
        Image,
        Component,
        Play,
        Plus,
        AlignStartVertical,
        AlignCenterVertical,
        AlignEndVertical,
        AlignStartHorizontal,
        AlignCenterHorizontal,
        AlignEndHorizontal,
        RotateCw,
        Radius,
        CircleCheck,
        CircleAlert,
    ]
);

/// Serves the editor icons first, then the default icons of the kit components.
pub struct AppAssets;

impl AssetSource for AppAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        match EditorIcons.load(path)? {
            Some(data) => Ok(Some(data)),
            None => Assets.load(path),
        }
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut paths = EditorIcons.list(path)?;
        paths.extend(Assets.list(path)?);
        Ok(paths)
    }
}

/// Inter 4.1 (SIL OFL, see `assets/fonts/LICENSE-Inter.txt`).
pub fn fonts() -> Vec<Cow<'static, [u8]>> {
    vec![
        Cow::Borrowed(include_bytes!("../assets/fonts/Inter-Regular.ttf").as_slice()),
        Cow::Borrowed(include_bytes!("../assets/fonts/Inter-Medium.ttf").as_slice()),
        Cow::Borrowed(include_bytes!("../assets/fonts/Inter-SemiBold.ttf").as_slice()),
        Cow::Borrowed(include_bytes!("../assets/fonts/Inter-ExtraBold.ttf").as_slice()),
    ]
}
