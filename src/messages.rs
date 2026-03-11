#[derive(PartialEq, Debug, Clone, Copy)]
pub enum Message {

    Next,

    NewSong,

    TryAgain,

    Init,

    #[allow(dead_code, reason = "this code may not be dead depending on features")]
    Play,

    Pause,

    PlayPause,

    ChangeVolume(f32),

    Bookmark,

    Quit,
}
