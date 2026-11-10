//! 菜单栏图标（代码生成，无外部图像依赖）与标题格式化。

/// 图标边长。
const SIZE: u32 = 32;

/// 32×32 RGBA 像素，编译期生成：圆角方块轮廓 + 三条横线，形似键盘。
/// macOS 用白色模板图标，其他平台用主色蓝以适配浅色面板。
pub const ICON_RGBA: (u32, u32, &[u8]) = (SIZE, SIZE, &build_icon(icon_rgb()));

/// 图标颜色：macOS 模板图标为纯白，其他平台用仪表盘主色 `#3987e5`。
const fn icon_rgb() -> [u8; 3] {
    #[cfg(target_os = "macos")]
    {
        [255, 255, 255]
    }
    #[cfg(not(target_os = "macos"))]
    {
        [0x39, 0x87, 0xe5]
    }
}

/// 生成图标像素：`rgb` 为不透明区域颜色，alpha=0 为透明。
const fn build_icon(rgb: [u8; 3]) -> [u8; (SIZE * SIZE * 4) as usize] {
    let mut buf = [0u8; (SIZE * SIZE * 4) as usize];
    let mut y = 0u32;
    while y < SIZE {
        let mut x = 0u32;
        while x < SIZE {
            let visible = is_keyboard_pixel(x, y);
            let idx = ((y * SIZE + x) * 4) as usize;
            buf[idx] = rgb[0];
            buf[idx + 1] = rgb[1];
            buf[idx + 2] = rgb[2];
            buf[idx + 3] = if visible { 255 } else { 0 };
            x += 1;
        }
        y += 1;
    }
    buf
}

/// 判断像素是否属于图形：外框圆角矩形 + 内部两条横线挖空。
const fn is_keyboard_pixel(x: u32, y: u32) -> bool {
    // 外框范围（留 2px 边距）
    let in_outer = x >= 2 && x < SIZE - 2 && y >= 6 && y < SIZE - 6;
    if !in_outer {
        return false;
    }
    // 圆角：四角 2×2 剔除
    let corner = (x < 4 || x >= SIZE - 4) && (y < 8 || y >= SIZE - 8);
    if corner {
        return false;
    }
    // 内部横线镂空，形成键盘键位感
    let inner_y = y >= 9 && y < SIZE - 9;
    let on_line = inner_y && ((y == 12 || y == 13) || (y == 19 || y == 20));
    !on_line
}

/// 千分位格式化。
pub fn format_thousands(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

/// 菜单栏标题：键盘符号 + 今日汉字数。
pub fn menu_bar_title(today_chars: u64) -> String {
    format!("⌨ {}", format_thousands(today_chars))
}

/// Linux 菜单首项：今日汉字数（AppIndicator 不支持 macOS 式标题）。
pub fn today_menu_text(today_chars: u64) -> String {
    format!("今日 {} 字", format_thousands(today_chars))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_thousands() {
        assert_eq!(format_thousands(0), "0");
        assert_eq!(format_thousands(7), "7");
        assert_eq!(format_thousands(999), "999");
        assert_eq!(format_thousands(1000), "1,000");
        assert_eq!(format_thousands(1234), "1,234");
        assert_eq!(format_thousands(1234567), "1,234,567");
    }

    #[test]
    fn builds_menu_bar_title() {
        assert_eq!(menu_bar_title(0), "⌨ 0");
        assert_eq!(menu_bar_title(1234), "⌨ 1,234");
    }

    #[test]
    fn formats_today_menu_text() {
        assert_eq!(today_menu_text(0), "今日 0 字");
        assert_eq!(today_menu_text(1234), "今日 1,234 字");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_icon_uses_accent_color() {
        let (_, _, rgba) = ICON_RGBA;
        let px = rgba.chunks_exact(4).find(|p| p[3] > 200).unwrap();
        assert_eq!(&px[..3], &[0x39, 0x87, 0xe5]);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_icon_is_white_template() {
        let (_, _, rgba) = ICON_RGBA;
        let px = rgba.chunks_exact(4).find(|p| p[3] > 200).unwrap();
        assert_eq!(&px[..3], &[255, 255, 255]);
    }

    #[test]
    fn icon_buffer_matches_dimensions() {
        let (w, h, rgba) = ICON_RGBA;
        assert_eq!(w, 32);
        assert_eq!(h, 32);
        assert_eq!(
            rgba.len(),
            (w * h * 4) as usize,
            "RGBA 缓冲区大小必须为 w*h*4"
        );
    }

    #[test]
    #[allow(clippy::chunks_exact_to_as_chunks)] // 保持 brief 中的测试代码逐字不变
    fn icon_has_visible_and_transparent_pixels() {
        let (_, _, rgba) = ICON_RGBA;
        let alphas: Vec<u8> = rgba.chunks_exact(4).map(|px| px[3]).collect();
        assert!(alphas.iter().any(|&a| a > 200), "应存在不透明像素");
        assert!(
            alphas.iter().any(|&a| a < 20),
            "应存在透明像素（模板图标需要镂空）"
        );
    }
}
