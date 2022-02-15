use {
    gtk::{
        gdk::{EventMask, ScrollDirection},
        gdk_pixbuf::{InterpType, Pixbuf},
        gio::prelude::*,
        glib::clone,
        prelude::*,
    },
    serde::Deserialize,
    std::{cell::Cell, io::Cursor, process::Command, rc::Rc},
};

macro_rules! get_obj {
    ($builder:expr, $id:expr) => {
        // Catch and panic manually to get useful file and line info
        {
            use gtk::prelude::BuilderExtManual;
            match $builder.object($id) {
                Some(o) => o,
                None => panic!("could not get {}", $id),
            }
        }
    };
}

const LAYOUT: &str = include_str!("../boomer.glade");
const QUIT_KEY: u16 = 9;
const HIGHLIGHT_KEY: u16 = 50;
const SCALE_DELTA: f64 = 0.1;
const SCALE_MAX: f64 = 3.0;
const BACKGROUND: (f64, f64, f64) = (0.1, 0.1, 0.1);
const HIGHLIGHT_RADIUS: f64 = 70.0;
const HIGHLIGHT_STYLE: (f64, f64, f64, f64) = (1.0, 1.0, 1.0, 0.4);

#[derive(Debug, Deserialize)]
struct Output {
    name: String,
    focused: bool,
}

#[derive(Debug, giftwrap::Wrap)]
enum Error {
    Io(std::io::Error),
    Json(serde_json::Error),
    #[noWrap]
    NoOutput,
}

#[derive(Debug, Clone)]
struct ImageState {
    scale: Cell<f64>,
    offset: Cell<(f64, f64)>,
    mouse_pos: Cell<(f64, f64)>,
    highlight: Cell<bool>,
}

fn activate(app: &gtk::Application, img: Vec<u8>) {
    let builder = gtk::Builder::from_string(LAYOUT);

    let window: gtk::ApplicationWindow = get_obj!(builder, "main-window");
    window.set_application(Some(app));
    window.add_events(
        EventMask::SCROLL_MASK
            | EventMask::SMOOTH_SCROLL_MASK
            | EventMask::BUTTON_MOTION_MASK
            | EventMask::POINTER_MOTION_MASK,
    );

    let source_pixbuf = Pixbuf::from_read(Cursor::new(img.clone())).unwrap();
    let state = Rc::new(ImageState {
        scale: Cell::new(1f64),
        offset: Cell::new((0f64, 0f64)),
        mouse_pos: Cell::new((0f64, 0f64)),
        highlight: Cell::new(false),
    });

    let glarea: gtk::GLArea = get_obj!(builder, "gl-area");
    glarea.connect_draw(
        clone!(@strong img, @strong state => move |_, ctx| {
            let scale = state.scale.get();
            let (xpos, ypos) = state.offset.get();

            // TODO: Try usin `scale` instead for better performance
            if let Some(new_pb) = source_pixbuf.scale_simple((source_pixbuf.width() as f64 * scale) as i32, (source_pixbuf.height() as f64 * scale) as i32, InterpType::Nearest) {

                let pb_width = source_pixbuf.width() as f64;
                let pb_height = source_pixbuf.width() as f64;

                let new_pb_width = new_pb.width() as f64;
                let new_pb_height = new_pb.width() as f64;

                let x = -(new_pb_width - pb_width) / 2.0;
                let y = -(new_pb_height - pb_height) / 2.0;

                // Fill background
                ctx.set_source_rgba(BACKGROUND.0, BACKGROUND.1, BACKGROUND.1, 1f64);
                let _ = ctx.paint();
                // Paint pixbuf
                ctx.set_source_pixbuf(&new_pb, x - xpos, y - ypos);
                let _= ctx.paint();

                if state.highlight.get() {
                    let (mx, my) = state.mouse_pos.get();
                    ctx.set_source_rgba(HIGHLIGHT_STYLE.0, HIGHLIGHT_STYLE.1, HIGHLIGHT_STYLE.2, HIGHLIGHT_STYLE.3);
                    ctx.arc(mx, my, HIGHLIGHT_RADIUS, 0.0, std::f64::consts::TAU);
                    let _ = ctx.fill();
                }

            }
            Inhibit(true)
        }),
    );

    window.connect_key_press_event(
        clone!(@strong glarea, @strong app, @strong state => move |_, evt| {
            match evt.keycode() {
                Some(QUIT_KEY) => app.quit(),
                Some(HIGHLIGHT_KEY) => {
                    state.highlight.set(true);
                    glarea.queue_render();
                },
                _ => {}
            }
            Inhibit(false)
        }),
    );

    window.connect_key_release_event(clone!(@strong glarea, @strong state => move |_, evt| {
        if let Some(HIGHLIGHT_KEY) = evt.keycode() {
            state.highlight.set(false);
            glarea.queue_render();
        }
        Inhibit(false)
    }));

    window.connect_scroll_event(clone!(@strong state, @strong glarea => move |_, evt| {
        match evt.direction() {
            ScrollDirection::Up => {
                state.scale.set((state.scale.get() + SCALE_DELTA).min(SCALE_MAX));
                glarea.queue_render();
            },
            ScrollDirection::Down => {
                state.scale.set((state.scale.get() - SCALE_DELTA).max(SCALE_DELTA));
                glarea.queue_render();
            },
            _ => {}
        }
        Inhibit(false)
    }));

    static mut LAST_POS: Option<(f64, f64)> = None;
    window.connect_motion_notify_event(clone!(@strong state, @strong glarea => move |_, evt| {
        let pos = evt.position();
        state.mouse_pos.set(pos);
        if evt.state().contains(gtk::gdk::ModifierType::BUTTON1_MASK) {
            if let Some(lp) = unsafe { LAST_POS } {
                let (xoff, yoff) = state.offset.get();
                state.offset.set((xoff + lp.0 - pos.0, yoff + lp.1 - pos.1));
                glarea.queue_render();
            }
            unsafe { LAST_POS = Some(pos) };
        }

        if state.highlight.get() {
            glarea.queue_render();
        }

        Inhibit(false)
    }));

    window.connect_button_release_event(clone!(@strong glarea => move |_, _| {
        unsafe { LAST_POS = None };
        Inhibit(false)
    }));

    gtk_layer_shell::init_for_window(&window);
    gtk_layer_shell::set_layer(&window, gtk_layer_shell::Layer::Overlay);
    gtk_layer_shell::set_keyboard_interactivity(&window, true);

    [
        (gtk_layer_shell::Edge::Left, true),
        // (gtk_layer_shell::Edge::Left, false),
        (gtk_layer_shell::Edge::Right, true),
        (gtk_layer_shell::Edge::Top, true),
        (gtk_layer_shell::Edge::Bottom, true),
    ]
    .iter()
    .for_each(|(anchor, state)| {
        gtk_layer_shell::set_anchor(&window, *anchor, *state);
    });

    window.show_all();
    // window.fullscreen()
}

fn main() -> Result<(), Error> {
    let output = serde_json::from_slice::<Vec<Output>>(
        &Command::new("swaymsg")
            .args(&["-t", "get_outputs", "-r"])
            .output()?
            .stdout,
    )?
    .into_iter()
    .filter_map(|o| match o.focused {
        true => Some(o.name),
        false => None,
    })
    .next()
    .ok_or(Error::NoOutput)?;

    let img = Command::new("grim")
        .args(&["-o", &output, "-"])
        .output()?
        .stdout;

    println!("Monitor: {}", output);

    let application = gtk::Application::new(
        Some(concat!("net.olback.", env!("CARGO_PKG_NAME"))),
        Default::default(),
    );

    application.connect_activate(move |app| {
        activate(app, img.clone());
    });

    application.run();

    Ok(())
}
