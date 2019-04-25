use crate::edit::EditMode;
use crate::sandbox::SandboxMode;
use crate::state::{Flags, UIState};
use crate::tutorial::TutorialMode;
use crate::ui::UI;
use abstutil::elapsed_seconds;
use ezgui::{
    Canvas, EventCtx, EventLoopMode, GfxCtx, LogScroller, ModalMenu, Prerender, TopMenu, UserInput,
    Wizard, GUI,
};
use geom::{Duration, Line, Pt2D, Speed};
use map_model::Map;
use rand::Rng;
use rand_xorshift::XorShiftRng;
use std::path::PathBuf;
use std::time::Instant;

// This is the top-level of the GUI logic. This module should just manage interactions between the
// top-level game states.
pub struct GameState {
    pub mode: Mode,
    pub ui: UI,
}

pub enum Mode {
    SplashScreen(Wizard, Option<(Screensaver, XorShiftRng)>),
    Legacy,
    Edit(EditMode),
    Tutorial(TutorialMode),
    Sandbox(SandboxMode),
}

impl GameState {
    pub fn new(flags: Flags, canvas: &mut Canvas, prerender: &Prerender) -> GameState {
        let splash = !flags.no_splash;
        let mut rng = flags.sim_flags.make_rng();
        let mut game = GameState {
            mode: Mode::Legacy,
            ui: UI::new(UIState::new(flags, prerender, true), canvas),
        };
        if splash {
            game.mode = Mode::SplashScreen(
                Wizard::new(),
                Some((
                    Screensaver::start_bounce(&mut rng, canvas, &game.ui.state.primary.map),
                    rng,
                )),
            );
        }
        game
    }
}

impl GUI for GameState {
    // TODO Totally get rid of this...
    fn top_menu(&self, canvas: &Canvas) -> Option<TopMenu> {
        self.ui.top_menu(canvas)
    }

    fn modal_menus(&self) -> Vec<ModalMenu> {
        self.ui.modal_menus()
    }

    fn event(&mut self, ctx: &mut EventCtx) -> EventLoopMode {
        match self.mode {
            Mode::SplashScreen(ref mut wizard, ref mut maybe_screensaver) => {
                if let Some((ref mut screensaver, ref mut rng)) = maybe_screensaver {
                    screensaver.update(rng, ctx.input, ctx.canvas, &self.ui.state.primary.map);
                }

                if let Some(new_mode) = splash_screen(wizard, ctx, &mut self.ui, maybe_screensaver)
                {
                    self.mode = new_mode;
                } else if wizard.aborted() {
                    self.ui.before_quit(ctx.canvas);
                    std::process::exit(0);
                }
                EventLoopMode::Animation
            }
            Mode::Legacy => {
                let (event_mode, pause) = self.ui.new_event(ctx);
                if pause {
                    self.mode = Mode::SplashScreen(Wizard::new(), None);
                }
                event_mode
            }
            Mode::Edit(_) => EditMode::event(self, ctx),
            Mode::Tutorial(_) => TutorialMode::event(self, ctx),
            Mode::Sandbox(_) => SandboxMode::event(self, ctx),
        }
    }

    fn draw(&self, g: &mut GfxCtx) {
        match self.mode {
            Mode::SplashScreen(ref wizard, _) => {
                self.ui.draw(g);
                wizard.draw(g);
            }
            Mode::Legacy => self.ui.draw(g),
            Mode::Edit(_) => EditMode::draw(self, g),
            Mode::Tutorial(_) => TutorialMode::draw(self, g),
            Mode::Sandbox(_) => SandboxMode::draw(self, g),
        }
    }

    fn dump_before_abort(&self, canvas: &Canvas) {
        self.ui.dump_before_abort(canvas);
    }

    fn before_quit(&self, canvas: &Canvas) {
        self.ui.before_quit(canvas);
    }

    fn profiling_enabled(&self) -> bool {
        self.ui.profiling_enabled()
    }
}

const SPEED: Speed = Speed::const_meters_per_second(20.0);

pub struct Screensaver {
    line: Line,
    started: Instant,
}

impl Screensaver {
    fn start_bounce(rng: &mut XorShiftRng, canvas: &mut Canvas, map: &Map) -> Screensaver {
        let at = canvas.center_to_map_pt();
        let bounds = map.get_bounds();
        // TODO Ideally bounce off the edge of the map
        let goto = Pt2D::new(
            rng.gen_range(0.0, bounds.max_x),
            rng.gen_range(0.0, bounds.max_y),
        );

        canvas.cam_zoom = 10.0;
        canvas.center_on_map_pt(at);

        Screensaver {
            line: Line::new(at, goto),
            started: Instant::now(),
        }
    }

    fn update(
        &mut self,
        rng: &mut XorShiftRng,
        input: &mut UserInput,
        canvas: &mut Canvas,
        map: &Map,
    ) {
        if input.nonblocking_is_update_event() {
            input.use_update_event();
            let dist_along = Duration::seconds(elapsed_seconds(self.started)) * SPEED;
            if dist_along < self.line.length() {
                canvas.center_on_map_pt(self.line.dist_along(dist_along));
            } else {
                *self = Screensaver::start_bounce(rng, canvas, map)
            }
        }
    }
}

fn splash_screen(
    raw_wizard: &mut Wizard,
    ctx: &mut EventCtx,
    ui: &mut UI,
    maybe_screensaver: &mut Option<(Screensaver, XorShiftRng)>,
) -> Option<Mode> {
    let mut wizard = raw_wizard.wrap(&mut ctx.input, ctx.canvas);
    let sandbox = "Sandbox mode";
    let load_map = "Load another map";
    let edit = "Edit map";
    let tutorial = "Tutorial";
    let legacy = "Legacy mode (ignore this)";
    let about = "About";
    let quit = "Quit";

    // Loop because we might go from About -> top-level menu repeatedly, and recursion is scary.
    loop {
        match wizard
            .choose_string(
                "Welcome to A/B Street!",
                vec![sandbox, load_map, edit, tutorial, legacy, about, quit],
            )?
            .as_str()
        {
            x if x == sandbox => break Some(Mode::Sandbox(SandboxMode::new())),
            x if x == load_map => {
                let current_map = ui.state.primary.map.get_name().to_string();
                if let Some((name, _)) = wizard.choose_something::<String>(
                    "Load which map?",
                    Box::new(move || {
                        abstutil::list_all_objects("maps", "")
                            .into_iter()
                            .filter(|(n, _)| n != &current_map)
                            .collect()
                    }),
                ) {
                    // This retains no state, but that's probably fine.
                    let mut flags = ui.state.primary.current_flags.clone();
                    flags.sim_flags.load = PathBuf::from(format!("../data/maps/{}.abst", name));
                    *ui = UI::new(UIState::new(flags, ctx.prerender, true), ctx.canvas);
                    break Some(Mode::Sandbox(SandboxMode::new()));
                } else if wizard.aborted() {
                    break Some(Mode::SplashScreen(Wizard::new(), maybe_screensaver.take()));
                } else {
                    break None;
                }
            }
            x if x == edit => break Some(Mode::Edit(EditMode::ViewingDiffs)),
            x if x == tutorial => {
                break Some(Mode::Tutorial(TutorialMode::Part1(
                    ctx.canvas.center_to_map_pt(),
                )))
            }
            x if x == legacy => break Some(Mode::Legacy),
            x if x == about => {
                if wizard.acknowledge(LogScroller::new(
                    "About A/B Street".to_string(),
                    vec![
                        "Author: Dustin Carlino (dabreegster@gmail.com)".to_string(),
                        "http://github.com/dabreegster/abstreet".to_string(),
                        "Map data from OpenStreetMap and King County GIS".to_string(),
                        "".to_string(),
                        "Press ENTER to continue".to_string(),
                    ],
                )) {
                    continue;
                } else {
                    break None;
                }
            }
            x if x == quit => {
                ui.before_quit(ctx.canvas);
                std::process::exit(0);
            }
            _ => unreachable!(),
        }
    }
}