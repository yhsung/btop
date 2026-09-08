/// One mouse mapping (`Input::mouse_mappings` entry, btop_input.cpp).
/// `x/y` is the top-left cell, `w/h` the size, `action` the key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MouseMap {
    pub x: i64,
    pub y: i64,
    pub w: i64,
    pub h: i64,
    pub action: String,
}
