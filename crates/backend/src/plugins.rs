//! The built-in plugins, one per plugin crate, in their order of registration: the list the
//! composition root registers. A plugin leaves a deployment by leaving this list.

use demi_plugin_audio::Audio;
use demi_plugin_browser::Browser;
use demi_plugin_changes::Changes;
use demi_plugin_file::File;
use demi_plugin_file_browser::FileBrowser;
use demi_plugin_image::Image;
use demi_plugin_video::Video;
use demi_plugin_interface::PluginFactory;
use demi_plugin_mobile::Mobile;
use demi_plugin_skills::Skills;
use demi_plugin_web::Web;

/// The plugins built into the backend.
pub fn builtin() -> Vec<Box<dyn PluginFactory>> {
    vec![
        Box::new(File::new()),
        Box::new(Browser::new()),
        Box::new(Mobile::new()),
        Box::new(Skills::new()),
        Box::new(Web::new()),
        Box::new(Image::new()),
        Box::new(Video::new()),
        Box::new(Audio::new()),
        Box::new(Changes::new()),
        Box::new(FileBrowser::new()),
    ]
}
