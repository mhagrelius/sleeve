use gtk::prelude::*;
use sleeve::ui::SleeveApplication;

fn main() -> gtk::glib::ExitCode {
    gtk::glib::set_application_name("Sleeve");
    gtk::glib::set_prgname(Some(sleeve::APP_ID));
    SleeveApplication::new().run()
}
