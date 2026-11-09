//! 菜单栏入口与事件循环。

use std::path::PathBuf;
use std::time::Duration;

use tao::event::Event;
use tao::event_loop::{ControlFlow, EventLoopBuilder};
use tao::platform::macos::{ActivationPolicy, EventLoopExtMacOS};
use tray_icon::menu::{Menu, MenuEvent, MenuItem};
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

use crate::icons::{menu_bar_title, ICON_RGBA};
use crate::store::Store;
use crate::sync::{sync_once, SyncState};

/// 同步线程唤醒 UI 的事件。
#[derive(Debug, Clone, Copy)]
pub enum UserEvent {
    /// 已同步完毕，请刷新菜单栏数字
    Synced,
}

const SYNC_INTERVAL: Duration = Duration::from_secs(60);

fn open_in_browser(port: u16) {
    let url = format!("http://127.0.0.1:{port}/");
    let _ = std::process::Command::new("open").arg(url).spawn();
}

fn open_data_dir(dir: &PathBuf) {
    let _ = std::process::Command::new("open").arg(dir).spawn();
}

fn build_icon() -> Icon {
    let (w, h, rgba) = ICON_RGBA;
    Icon::from_rgba(rgba.to_vec(), w, h).expect("内嵌图标必须是合法 RGBA")
}

/// 菜单栏标题所用的今日字数。
fn today_chars(state: &SyncState) -> u64 {
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    state.characters_on(&today)
}

/// 运行菜单栏应用，永不返回。
pub fn run_app(rime_dir: PathBuf, data_dir: PathBuf, state: SyncState, port: u16) -> ! {
    let mut event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
    event_loop.set_activation_policy(ActivationPolicy::Accessory);

    let proxy = event_loop.create_proxy();

    // 后台同步线程：立即同步一次，之后按间隔循环
    {
        let state = state.clone();
        let proxy = proxy.clone();
        let rime_dir = rime_dir.clone();
        let data_dir = data_dir.clone();
        std::thread::spawn(move || {
            let Ok(store) = Store::open(&data_dir) else {
                return;
            };
            loop {
                let outcome = sync_once(&rime_dir, &store);
                state.replace(outcome);
                let _ = proxy.send_event(UserEvent::Synced);
                std::thread::sleep(SYNC_INTERVAL);
            }
        });
    }

    let mut tray: Option<TrayIcon> = None;
    let mut menu_open: Option<MenuItem> = None;
    let mut menu_sync: Option<MenuItem> = None;
    let mut menu_data_dir: Option<MenuItem> = None;
    let mut menu_quit: Option<MenuItem> = None;
    // 已写入菜单栏的标题。AppKit 的 setTitle 即使内容不变也会标记状态栏项为脏，
    // 触发重绘 → 产生事件 → 再次 setTitle，形成自持重绘循环（实测吃掉 97% CPU）。
    // 因此标题只在同步完成且文本确实变化时才更新。
    let mut shown_title: Option<String> = None;

    event_loop.run(move |event, _, control_flow| {
        // 完全阻塞等待：周期同步由后台线程自己 sleep 驱动，事件循环不需要定时器
        *control_flow = ControlFlow::Wait;

        match event {
            Event::NewEvents(_) => {
                // macOS 要求事件循环已开始运行后才能创建 TrayIcon
                if tray.is_none() {
                    let menu = Menu::new();
                    let open = MenuItem::new("打开统计", true, None);
                    let sync_now = MenuItem::new("立即同步", true, None);
                    let data = MenuItem::new("打开数据目录", true, None);
                    let quit = MenuItem::new("退出", true, None);
                    let _ = menu.append(&open);
                    let _ = menu.append(&sync_now);
                    let _ = menu.append(&data);
                    let _ = menu.append(&quit);

                    let initial_title = menu_bar_title(today_chars(&state));
                    let icon = TrayIconBuilder::new()
                        .with_menu(Box::new(menu))
                        .with_menu_on_left_click(false)
                        .with_icon_as_template(true)
                        .with_icon(build_icon())
                        .with_tooltip("Rime 打字统计")
                        .with_title(&initial_title)
                        .build();

                    if let Ok(icon) = icon {
                        tray = Some(icon);
                        shown_title = Some(initial_title);
                        menu_open = Some(open);
                        menu_sync = Some(sync_now);
                        menu_data_dir = Some(data);
                        menu_quit = Some(quit);
                    }
                }
            }
            Event::MainEventsCleared => {
                // 托盘左键：打开网页
                while let Ok(ev) = TrayIconEvent::receiver().try_recv() {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = ev
                    {
                        open_in_browser(port);
                    }
                }

                // 菜单项
                while let Ok(MenuEvent { id }) = MenuEvent::receiver().try_recv() {
                    if Some(&id) == menu_quit.as_ref().map(|m| m.id()) {
                        *control_flow = ControlFlow::Exit;
                    } else if Some(&id) == menu_open.as_ref().map(|m| m.id()) {
                        open_in_browser(port);
                    } else if Some(&id) == menu_sync.as_ref().map(|m| m.id()) {
                        let rime_dir = rime_dir.clone();
                        let data_dir = data_dir.clone();
                        let state = state.clone();
                        std::thread::spawn(move || {
                            if let Ok(store) = Store::open(&data_dir) {
                                state.replace(sync_once(&rime_dir, &store));
                            }
                        });
                    } else if Some(&id) == menu_data_dir.as_ref().map(|m| m.id()) {
                        open_data_dir(&data_dir);
                    }
                }
            }
            Event::UserEvent(UserEvent::Synced) => {
                // 同步完成后才刷新标题。这是标题唯一的更新时机：
                // AppKit 的 setTitle 即使文本不变也会把状态栏项标记为脏并触发整条
                // 重绘链路，若放在 MainEventsCleared 里无条件调用，会形成
                // 重绘 → 事件 → 再重绘 的自持循环（实测占满 97% 单核）。
                if let Some(icon) = tray.as_ref() {
                    let title = menu_bar_title(today_chars(&state));
                    if shown_title.as_deref() != Some(title.as_str()) {
                        icon.set_title(Some(title.clone()));
                        shown_title = Some(title);
                    }
                }
            }
            Event::LoopDestroyed => {
                *control_flow = ControlFlow::Exit;
            }
            _ => {}
        }
    })
}
