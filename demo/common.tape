# Shared look for every demo scene: render.sh puts this at the top of each
# scene before recording it.
# Matches the window-on-lavender style: macOS-style window bar, rounded
# corners, dark navy terminal.

Set Shell "zsh"
Set FontSize 15
Set Width 960
Set Height 520
Set Padding 20
Set Margin 22
Set MarginFill "#EEF0FB"
Set WindowBar Colorful
Set WindowBarSize 34
Set BorderRadius 10
Set Framerate 24
Set TypingSpeed 45ms
Set CursorBlink false
Set Theme { "name": "vaqum", "black": "#45475a", "red": "#f38ba8", "green": "#a6e3a1", "yellow": "#f9e2af", "blue": "#89b4fa", "magenta": "#cba6f7", "cyan": "#94e2d5", "white": "#bac2de", "brightBlack": "#585b70", "brightRed": "#f38ba8", "brightGreen": "#a6e3a1", "brightYellow": "#f9e2af", "brightBlue": "#89b4fa", "brightMagenta": "#cba6f7", "brightCyan": "#94e2d5", "brightWhite": "#cdd6f4", "background": "#0E1324", "foreground": "#cdd6f4", "selection": "#313244", "cursor": "#b4befe" }

# Swap into a fresh zsh that loads only the demo config from demo/setup.sh
# (prompt, PATH, $HOME), so your own shell theme and installed vaqum never
# leak into the recording. Then start each scene on a clean screen.
Hide
Sleep 500ms
Type "exec env ZDOTDIR=/tmp/vaqum-demo-zsh zsh -i"
Enter
Sleep 700ms
Type "clear"
Enter
Sleep 300ms
Show
Sleep 400ms
