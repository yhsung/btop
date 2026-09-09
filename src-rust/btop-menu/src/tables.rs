//! Static menu tables transcribed from `src/btop_menu.cpp` (see line refs per item).
//!
//! Scope: Apple Silicon + `GPU_SUPPORT` build — the `__APPLE__` `P_Signals`
//! branch, all `#ifdef GPU_SUPPORT` blocks included, `#ifdef __linux__`
//! blocks (`freq_mode`) excluded. Bytes are verbatim from source.

/// Signal names for the signal-choose menu (`btop_menu.cpp:64-132`).
/// Index 0 is `"0"`; the remaining 31 are the `__APPLE__` branch (`:114-121`).
pub const P_SIGNALS: [&str; 32] = [
    "0",
    "SIGHUP",
    "SIGINT",
    "SIGQUIT",
    "SIGILL",
    "SIGTRAP",
    "SIGABRT",
    "SIGEMT",
    "SIGFPE",
    "SIGKILL",
    "SIGBUS",
    "SIGSEGV",
    "SIGSYS",
    "SIGPIPE",
    "SIGALRM",
    "SIGTERM",
    "SIGURG",
    "SIGSTOP",
    "SIGTSTP",
    "SIGCONT",
    "SIGCHLD",
    "SIGTTIN",
    "SIGTTOU",
    "SIGIO",
    "SIGXCPU",
    "SIGXFSZ",
    "SIGVTALRM",
    "SIGPROF",
    "SIGWINCH",
    "SIGINFO",
    "SIGUSR1",
    "SIGUSR2",
];

/// Help window rows (`btop_menu.cpp:174-221`): `(key, description)`.
pub const HELP_TEXT: &[(&str, &str)] = &[
    ("Mouse 1", "Clicks buttons and selects in process list."), // :175
    ("Mouse scroll", "Scrolls any scrollable list/text under cursor."), // :176
    ("Esc, m", "Toggles main menu."), // :177
    ("p", "Cycle view presets forwards."), // :178
    ("shift + p", "Cycle view presets backwards."), // :179
    ("1", "Toggle CPU box."), // :180
    ("2", "Toggle MEM box."), // :181
    ("3", "Toggle NET box."), // :182
    ("4", "Toggle PROC box."), // :183
    ("5", "Toggle GPU box."), // :184
    ("d", "Toggle disks view in MEM box."), // :185
    ("F2, o", "Shows options."), // :186
    ("F1, ?, h", "Shows this window."), // :187
    ("ctrl + z", "Sleep program and put in background."), // :188
    ("ctrl + r", "Reloads config file from disk."), // :189
    ("q, ctrl + c", "Quits program."), // :190
    ("+, -", "Add/Subtract 100ms to/from update timer."), // :191
    ("Up, Down", "Select in process list."), // :192
    ("Enter", "Show detailed information for selected process."), // :193
    ("Spacebar", "Expand/collapse the selected process in tree view."), // :194
    ("C", "Expand/collapse the selected process' children."), // :195
    ("Pg Up, Pg Down", "Jump 1 page in process list."), // :196
    ("Home, End", "Jump to first or last page in process list."), // :197
    ("Left, Right", "Select previous/next sorting column."), // :198
    ("b, n", "Select previous/next network device."), // :199
    ("i", "Toggle disks io mode with big graphs."), // :200
    ("z", "Toggle totals reset for current network device"), // :201
    ("a", "Toggle auto scaling for the network graphs."), // :202
    ("y", "Toggle synced scaling mode for network graphs."), // :203
    ("f, /", "To enter a process filter. Start with ! for regex."), // :204
    ("F", "Follow selected process."), // :205
    ("u", "Pause process list."), // :206
    ("delete", "Clear any entered filter."), // :207
    ("c", "Toggle per-core cpu usage of processes."), // :208
    ("r", "Reverse sorting order in processes box."), // :209
    ("e", "Toggle processes tree view."), // :210
    ("E", "Collapse/expand all processes in tree view."), // :211
    ("%", "Toggles memory display mode in processes box."), // :212
    ("Selected +, -", "Expand/collapse the selected process in tree view."), // :213
    ("Selected t", "Terminate selected process with SIGTERM - 15."), // :214
    ("Selected k", "Kill selected process with SIGKILL - 9."), // :215
    ("Selected s", "Select or enter signal to send to process."), // :216
    ("Selected N", "Select new nice value for selected process."), // :217
    ("", " "), // :218
    ("", "For bug reporting and project updates, visit:"), // :219
    ("", "https://github.com/aristocratos/btop"), // :220
];

/// Menu banners (`btop_menu.cpp:136-170`), 3 lines per banner.
/// Indices 0-8 are `menu_normal` (btop, help, quit);
/// indices 9-17 are `menu_selected` (btop, help, quit).
pub const MENU_BANNERS: &[&str] = &[
    "┌─┐┌─┐┌┬┐┬┌─┐┌┐┌┌─┐", // :138
    "│ │├─┘ │ ││ ││││└─┐", // :139
    "└─┘┴   ┴ ┴└─┘┘└┘└─┘", // :140
    "┬ ┬┌─┐┬  ┌─┐", // :143
    "├─┤├┤ │  ├─┘", // :144
    "┴ ┴└─┘┴─┘┴  ", // :145
    "┌─┐ ┬ ┬ ┬┌┬┐", // :148
    "│─┼┐│ │ │ │ ", // :149
    "└─┘└└─┘ ┴ ┴ ", // :150
    "╔═╗╔═╗╔╦╗╦╔═╗╔╗╔╔═╗", // :156
    "║ ║╠═╝ ║ ║║ ║║║║╚═╗", // :157
    "╚═╝╩   ╩ ╩╚═╝╝╚╝╚═╝", // :158
    "╦ ╦╔═╗╦  ╔═╗", // :161
    "╠═╣╠╣ ║  ╠═╝", // :162
    "╩ ╩╚═╝╩═╝╩  ", // :163
    "╔═╗ ╦ ╦ ╦╔╦╗ ", // :166
    "║═╬╗║ ║ ║ ║  ", // :167
    "╚═╝╚╚═╝ ╩ ╩  ", // :168
];

/// Option categories (`btop_menu.cpp:223-907`): `categories[tab][option] = [name, desc...]`.
/// Tab 0 = general (21 options).
/// Tab 1 = cpu (16 options).
/// Tab 2 = gpu (11 options).
/// Tab 3 = mem (16 options).
/// Tab 4 = net (8 options).
/// Tab 5 = proc (15 options).
pub const CATEGORIES: &[&[&[&str]]] = &[
    // Tab 0: general
    &[
        &["color_theme", "Set color theme.", "", "Choose from all theme files in (usually)", "\"/usr/[local/]share/btop/themes\" and", "\"~/.config/btop/themes\".", "", "\"Default\" for builtin default theme.", "\"TTY\" for builtin 16-color theme.", "", "For theme updates see:", "https://github.com/aristocratos/btop"], // :225 color_theme
        &["theme_background", "If the theme set background should be shown.", "", "Set to False if you want terminal background", "transparency."], // :237 theme_background
        &["truecolor", "Sets if 24-bit truecolor should be used.", "", "Will convert 24-bit colors to 256 color", "(6x6x6 color cube) if False.", "", "Set to False if your terminal doesn't have", "truecolor support and can't convert to", "256-color."], // :242 truecolor
        &["force_tty", "TTY mode.", "", "Set to true to force tty mode regardless", "if a real tty has been detected or not.", "", "Will force 16-color mode and TTY theme,", "set all graph symbols to \"tty\" and swap", "out other non tty friendly symbols."], // :251 force_tty
        &["vim_keys", "Enable vim keys.", "Set to True to enable \"h,j,k,l\" keys for", "directional control in lists.", "", "Conflicting keys for", "h (help) and k (kill)", "is accessible while holding shift."], // :260 vim_keys
        &["disable_mouse", "Disable all mouse events."], // :268 disable_mouse
        &["disable_presets", "Disable the presets.", "", "\"Off\" All presets are enabled.", "", "\"Default\" preset is disabled.", "", "\"Custom\" presets are disabled.", "", "\"All\" presets are disabled."], // :270 disable_presets
        &["presets", "Define presets for the layout of the boxes.", "", "Preset 0 is always all boxes shown with", "default settings.", "Max 9 presets.", "", "Format: \"box_name:P:G,box_name:P:G\"", "P=(0 or 1) for alternate positions.", "G=graph symbol to use for box.", "", "Use whitespace \" \" as separator between", "different presets.", "", "Example:", "\"mem:0:tty,proc:1:default cpu:0:braille\""], // :280 presets
        &["shown_boxes", "Manually set which boxes to show.", "", "Available values are \"cpu mem net proc\".", "Or \"gpu0\" through \"gpu5\" for GPU boxes.", "Separate values with whitespace.", "", "Toggle between presets with key \"p\"."], // :296 shown_boxes
        &["update_ms", "Update time in milliseconds.", "", "Recommended 2000 ms or above for better", "sample times for graphs.", "", "Min value: 100 ms", "Max value: 86400000 ms = 24 hours."], // :304 update_ms
        &["rounded_corners", "Rounded corners on boxes.", "", "True or False", "", "Is always False if TTY mode is ON."], // :312 rounded_corners
        &["terminal_sync", "Output synchronization.", "", "Use terminal synchronized output sequences", "to reduce flickering on supported terminals.", "", "True or False."], // :318 terminal_sync
        &["graph_symbol", "Default symbols to use for graph creation.", "", "\"braille\", \"block\" or \"tty\".", "", "\"braille\" offers the highest resolution but", "might not be included in all fonts.", "", "\"block\" has half the resolution of braille", "but uses more common characters.", "", "\"tty\" uses only 3 different symbols but will", "work with most fonts.", "", "Note that \"tty\" only has half the horizontal", "resolution of the other two,", "so will show a shorter historical view."], // :325 graph_symbol
        &["clock_format", "Draw a clock at top of screen.", "(Only visible if cpu box is enabled!)", "", "Formatting according to strftime, empty", "string to disable.", "", "Custom formatting options:", "\"/host\" = hostname", "\"/user\" = username", "\"/uptime\" = system uptime", "", "Examples of strftime formats:", "\"%X\" = locale HH:MM:SS", "\"%H\" = 24h hour, \"%I\" = 12h hour", "\"%M\" = minute, \"%S\" = second", "\"%d\" = day, \"%m\" = month, \"%y\" = year"], // :342 clock_format
        &["base_10_sizes", "Use base 10 for bits and bytes sizes.", "", "Uses KB = 1000 instead of KiB = 1024,", "MB = 1000KB instead of MiB = 1024KiB,", "and so on.", "", "True or False."], // :359 base_10_sizes
        &["background_update", "Update main ui when menus are showing.", "", "True or False.", "", "Set this to false if the menus is flickering", "too much for a comfortable experience."], // :367 background_update
        &["show_battery", "Show battery stats.", "(Only visible if cpu box is enabled!)", "", "Show battery stats in the top right corner", "if a battery is present."], // :374 show_battery
        &["selected_battery", "Select battery.", "", "Which battery to use if multiple are present.", "Can be both batteries and UPS.", "", "\"Auto\" for auto detection."], // :380 selected_battery
        &["show_battery_watts", "Show battery power.", "", "Show discharge power when discharging.", "Show charging power when charging."], // :387 show_battery_watts
        &["log_level", "Set loglevel for error.log", "", "\"ERROR\", \"WARNING\", \"INFO\" and \"DEBUG\".", "", "The level set includes all lower levels,", "i.e. \"DEBUG\" will show all logging info."], // :392 log_level
        &["save_config_on_exit", "Save config on exit.", "", "Automatically save current settings to", "config file on exit.", "", "When this is toggled from True to False", "a save is immediately triggered.", "This way a manual save can be done by", "toggling this setting on and off again."], // :399 save_config_on_exit
    ],
    // Tab 1: cpu
    &[
        &["cpu_bottom", "Cpu box location.", "", "Show cpu box at bottom of screen instead", "of top."], // :411 cpu_bottom
        &["graph_symbol_cpu", "Graph symbol to use for graphs in cpu box.", "", "\"default\", \"braille\", \"block\" or \"tty\".", "", "\"default\" for the general default symbol."], // :416 graph_symbol_cpu
        &["cpu_graph_upper", "Cpu upper graph.", "", "Sets the CPU/GPU stat shown in upper half of", "the CPU graph.", "", "CPU:", "\"total\" = Total cpu usage. (Auto)", "\"user\" = User mode cpu usage.", "\"system\" = Kernel mode cpu usage.", "+ more depending on kernel.", "", "GPU:", "\"gpu-totals\" = GPU usage split by device.", "\"gpu-vram-totals\" = VRAM usage split by GPU.", "\"gpu-pwr-totals\" = Power usage split by GPU.", "\"gpu-average\" = Avg usage of all GPUs.", "\"gpu-vram-total\" = VRAM usage of all GPUs.", "\"gpu-pwr-total\" = Power usage of all GPUs.", "Not all stats are supported on all devices."], // :422 cpu_graph_upper
        &["cpu_graph_lower", "Cpu lower graph.", "", "Sets the CPU/GPU stat shown in lower half of", "the CPU graph.", "", "CPU:", "\"total\" = Total cpu usage.", "\"user\" = User mode cpu usage.", "\"system\" = Kernel mode cpu usage.", "+ more depending on kernel.", "", "GPU:", "\"gpu-totals\" = GPU usage split/device. (Auto)", "\"gpu-vram-totals\" = VRAM usage split by GPU.", "\"gpu-pwr-totals\" = Power usage split by GPU.", "\"gpu-average\" = Avg usage of all GPUs.", "\"gpu-vram-total\" = VRAM usage of all GPUs.", "\"gpu-pwr-total\" = Power usage of all GPUs.", "Not all stats are supported on all devices."], // :443 cpu_graph_lower
        &["cpu_invert_lower", "Toggles orientation of the lower CPU graph.", "", "True or False."], // :464 cpu_invert_lower
        &["cpu_single_graph", "Completely disable the lower CPU graph.", "", "Shows only upper CPU graph and resizes it", "to fit to box height.", "", "True or False."], // :468 cpu_single_graph
        &["show_gpu_info", "Show gpu info in cpu box.", "", "Toggles gpu stats in cpu box and the", "gpu graph (if \"cpu_graph_lower\" is set to", "\"Auto\").", "", "\"Auto\" to show when no gpu box is shown.", "\"On\" to always show.", "\"Off\" to never show."], // :475 show_gpu_info
        &["check_temp", "Enable cpu temperature reporting.", "", "True or False."], // :485 check_temp
        &["cpu_sensor", "Cpu temperature sensor.", "", "Select the sensor that corresponds to", "your cpu temperature.", "", "Set to \"Auto\" for auto detection."], // :489 cpu_sensor
        &["show_coretemp", "Show temperatures for cpu cores.", "", "Only works if check_temp is True and", "the system is reporting core temps."], // :496 show_coretemp
        &["cpu_core_map", "Custom mapping between core and coretemp.", "", "Can be needed on certain cpus to get correct", "temperature for correct core.", "", "Use lm-sensors or similar to see which cores", "are reporting temperatures on your machine.", "", "Format: \"X:Y\"", "X=core with wrong temp.", "Y=core with correct temp.", "Use space as separator between multiple", "entries.", "", "Example: \"4:0 5:1 6:3\""], // :501 cpu_core_map
        &["temp_scale", "Which temperature scale to use.", "", "Celsius, default scale.", "", "Fahrenheit, the american one.", "", "Kelvin, 0 = absolute zero, 1 degree change", "equals 1 degree change in Celsius.", "", "Rankine, 0 = absolute zero, 1 degree change", "equals 1 degree change in Fahrenheit."], // :517 temp_scale
        &["show_cpu_freq", "Show CPU frequency.", "", "Can cause slowdowns on systems with many", "cores and certain kernel versions."], // :529 show_cpu_freq
        &["custom_cpu_name", "Custom cpu model name in cpu percentage box.", "", "Empty string to disable."], // :534 custom_cpu_name
        &["show_uptime", "Shows the system uptime in the CPU box.", "", "Can also be shown in the clock by using", "\"/uptime\" in the formatting.", "", "True or False."], // :538 show_uptime
        &["show_cpu_watts", "Shows the CPU power consumption in watts.", "", "Requires running `make setcap` or", "`make setuid` or running with sudo.", "", "True or False."], // :545 show_cpu_watts
    ],
    // Tab 2: gpu
    &[
        &["nvml_measure_pcie_speeds", "Measure PCIe throughput on NVIDIA cards.", "", "May impact performance on certain cards.", "", "True or False."], // :554 nvml_measure_pcie_speeds
        &["rsmi_measure_pcie_speeds", "Measure PCIe throughput on AMD cards.", "", "May impact performance on certain cards.", "", "True or False."], // :560 rsmi_measure_pcie_speeds
        &["graph_symbol_gpu", "Graph symbol to use for graphs in gpu box.", "", "\"default\", \"braille\", \"block\" or \"tty\".", "", "\"default\" for the general default symbol."], // :566 graph_symbol_gpu
        &["gpu_mirror_graph", "Horizontally mirror the GPU graph.", "", "True or False."], // :572 gpu_mirror_graph
        &["shown_gpus", "Manually set which gpu vendors to show.", "", "Available values are", "\"nvidia\", \"amd\", \"intel\",", "and \"apple\".", "Separate values with whitespace.", "", "A restart is required to apply changes."], // :576 shown_gpus
        &["custom_gpu_name0", "Custom gpu0 model name in gpu stats box.", "", "Empty string to disable."], // :585 custom_gpu_name0
        &["custom_gpu_name1", "Custom gpu1 model name in gpu stats box.", "", "Empty string to disable."], // :589 custom_gpu_name1
        &["custom_gpu_name2", "Custom gpu2 model name in gpu stats box.", "", "Empty string to disable."], // :593 custom_gpu_name2
        &["custom_gpu_name3", "Custom gpu3 model name in gpu stats box.", "", "Empty string to disable."], // :597 custom_gpu_name3
        &["custom_gpu_name4", "Custom gpu4 model name in gpu stats box.", "", "Empty string to disable."], // :601 custom_gpu_name4
        &["custom_gpu_name5", "Custom gpu5 model name in gpu stats box.", "", "Empty string to disable."], // :605 custom_gpu_name5
    ],
    // Tab 3: mem
    &[
        &["mem_below_net", "Mem box location.", "", "Show mem box below net box instead of above."], // :611 mem_below_net
        &["graph_symbol_mem", "Graph symbol to use for graphs in mem box.", "", "\"default\", \"braille\", \"block\" or \"tty\".", "", "\"default\" for the general default symbol."], // :615 graph_symbol_mem
        &["mem_graphs", "Show graphs for memory values.", "", "True or False."], // :621 mem_graphs
        &["show_disks", "Split memory box to also show disks.", "", "True or False."], // :625 show_disks
        &["show_io_stat", "Toggle IO activity graphs.", "", "Show small IO graphs that for disk activity", "(disk busy time) when not in IO mode.", "", "True or False."], // :629 show_io_stat
        &["io_mode", "Toggles io mode for disks.", "", "Shows big graphs for disk read/write speeds", "instead of used/free percentage meters.", "", "True or False."], // :636 io_mode
        &["io_graph_combined", "Toggle combined read and write graphs.", "", "Only has effect if \"io mode\" is True.", "", "True or False."], // :643 io_graph_combined
        &["io_graph_speeds", "Set top speeds for the io graphs.", "", "Manually set which speed in MiB/s that", "equals 100 percent in the io graphs.", "(100 MiB/s by default).", "", "Format: \"device:speed\" separate disks with", "whitespace \" \".", "", "Example: \"/dev/sda:100, /dev/sdb:20\"."], // :649 io_graph_speeds
        &["show_swap", "If swap memory should be shown in memory box.", "", "True or False."], // :660 show_swap
        &["swap_disk", "Show swap as a disk.", "", "Ignores show_swap value above.", "Inserts itself after first disk."], // :664 swap_disk
        &["only_physical", "Filter out non physical disks.", "", "Set this to False to include network disks,", "RAM disks and similar.", "", "True or False."], // :669 only_physical
        &["use_fstab", "(Linux) Read disks list from /etc/fstab.", "", "This also disables only_physical.", "", "True or False."], // :676 use_fstab
        &["zfs_hide_datasets", "(Linux) Hide ZFS datasets in disks list.", "", "Setting this to True will hide all datasets,", "and only show ZFS pools.", "", "(IO stats will be calculated per-pool)", "", "True or False."], // :682 zfs_hide_datasets
        &["disk_free_priv", "(Linux) Type of available disk space.", "", "Set to true to show how much disk space is", "available for privileged users.", "", "Set to false to show available for normal", "users."], // :691 disk_free_priv
        &["disks_filter", "Optional filter for shown disks.", "", "Should be full path of a mountpoint.", "Separate multiple values with", "whitespace \" \".", "", "Only disks matching the filter will be shown.", "Prepend \u{1B}[3mexclude=\u{1B}[23m to only show disks ", "not matching the filter.", "", "Examples:", "/boot /home/user", "exclude=/boot /home/user"], // :699 disks_filter
        &["zfs_arc_cached", "(Linux) Count ZFS ARC as cached memory.", "", "Add ZFS ARC used to cached memory and", "ZFS ARC available to available memory.", "These are otherwise reported by the Linux", "kernel as used memory.", "", "True or False."], // :713 zfs_arc_cached
    ],
    // Tab 4: net
    &[
        &["graph_symbol_net", "Graph symbol to use for graphs in net box.", "", "\"default\", \"braille\", \"block\" or \"tty\".", "", "\"default\" for the general default symbol."], // :724 graph_symbol_net
        &["swap_upload_download", "Swap the positions of the upload and download", "graphs.", "", "This allows for a more \"intuitive\" view", "with download being down, on the bottom."], // :730 swap_upload_download
        &["net_download", "Fixed network graph download value.", "", "Value in Mebibits, default \"100\".", "", "Can be toggled with auto button."], // :736 net_download
        &["net_upload", "Fixed network graph upload value.", "", "Value in Mebibits, default \"100\".", "", "Can be toggled with auto button."], // :742 net_upload
        &["net_auto", "Start in network graphs auto rescaling mode.", "", "Ignores any values set above at start and", "rescales down to 10Kibibytes at the lowest.", "", "True or False."], // :748 net_auto
        &["net_sync", "Network scale sync.", "", "Syncs the scaling for download and upload to", "whichever currently has the highest scale.", "", "True or False."], // :755 net_sync
        &["net_iface", "Network Interface.", "", "Manually set the starting Network Interface.", "", "Will otherwise automatically choose the NIC", "with the highest total download since boot."], // :762 net_iface
        &["base_10_bitrate", "Base 10 bitrate", "", "True:  Use SI prefixes for bitrates", "       (1000Kbps = 1Mbps)", "False: Use binary prefixes for bitrates", "       (1024Kibps = 1Mibps)", "Auto:  Use the General -> Base 10 Sizes", "       setting for bitrates", "", "True, False, or Auto"], // :769 base_10_bitrate
    ],
    // Tab 5: proc
    &[
        &["proc_left", "Proc box location.", "", "Show proc box on left side of screen", "instead of right."], // :782 proc_left
        &["graph_symbol_proc", "Graph symbol to use for graphs in proc box.", "", "\"default\", \"braille\", \"block\" or \"tty\".", "", "\"default\" for the general default symbol."], // :787 graph_symbol_proc
        &["proc_sorting", "Processes sorting option.", "", "Possible values:", "\"pid\", \"program\", \"arguments\", \"threads\",", "\"user\", \"memory\", \"cpu lazy\" and", "\"cpu direct\".", "", "\"cpu lazy\" updates top process over time.", "\"cpu direct\" updates top process", "directly."], // :793 proc_sorting
        &["proc_reversed", "Reverse processes sorting order.", "", "True or False."], // :804 proc_reversed
        &["proc_tree", "Processes tree view.", "", "Set true to show processes grouped by", "parents with lines drawn between parent", "and child process."], // :808 proc_tree
        &["proc_aggregate", "Aggregate child's resources in parent.", "", "In tree-view, include all child resources", "with the parent even while expanded."], // :814 proc_aggregate
        &["proc_tree_auto_collapse", "Auto-collapse busy parents in tree view.", "", "When entering tree mode, automatically", "collapse any process that has this many", "or more direct children.", "", "Useful for hiding noisy multi-process apps", "like Chrome, Firefox or Electron.", "", "Set to 0 to disable.", "", "Min value: 0", "Max value: 10000"], // :819 proc_tree_auto_collapse
        &["proc_colors", "Enable colors in process view.", "", "True or False."], // :833 proc_colors
        &["proc_gradient", "Enable process view gradient fade.", "", "Fades from top or current selection.", "Max fade value is equal to current themes", "\"inactive_fg\" color value."], // :837 proc_gradient
        &["proc_per_core", "Process usage per core.", "", "If process cpu usage should be of the core", "it's running on or usage of the total", "available cpu power.", "", "If true and process is multithreaded", "cpu usage can reach over 100%."], // :843 proc_per_core
        &["proc_mem_bytes", "Show memory as bytes in process list.", " ", "Will show percentage of total memory", "if False."], // :852 proc_mem_bytes
        &["keep_dead_proc_usage", "Cpu and Mem usage for dead processes", "", "Set true if process should preserve the cpu", "and memory usage of when it died while", "paused."], // :857 keep_dead_proc_usage
        &["proc_cpu_graphs", "Show cpu graph for each process.", "", "True or False"], // :863 proc_cpu_graphs
        &["proc_filter_kernel", "(Linux) Filter kernel processes from output.", "", "Set to 'True' to filter out internal", "processes started by the Linux kernel."], // :867 proc_filter_kernel
        &["proc_follow_detailed", "Follow selected process with detailed view", "", "If set to 'True' then when opening the", "detailed view, the process will be", "followed in the list. Pressing enter", "again will close the detailed view", "and stop following the process."], // :872 proc_follow_detailed
    ],
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tab_counts_match_source() {
        assert_eq!(CATEGORIES.len(), 6);
        assert_eq!(CATEGORIES[0].len(), 21, "general tab");
        assert_eq!(CATEGORIES[1].len(), 16, "cpu tab");
        assert_eq!(CATEGORIES[2].len(), 11, "gpu tab");
        assert_eq!(CATEGORIES[3].len(), 16, "mem tab");
        assert_eq!(CATEGORIES[4].len(), 8, "net tab");
        assert_eq!(CATEGORIES[5].len(), 15, "proc tab");
    }

    #[test]
    fn table_sizes_match_source() {
        assert_eq!(P_SIGNALS.len(), 32);
        assert_eq!(HELP_TEXT.len(), 46);
        assert_eq!(MENU_BANNERS.len(), 18);
    }

    #[test]
    fn spot_entries() {
        assert_eq!(CATEGORIES[0][0][0], "color_theme");
        assert_eq!(CATEGORIES[1][11][0], "temp_scale");
        assert_eq!(P_SIGNALS[9], "SIGKILL");
        assert_eq!(HELP_TEXT[0], ("Mouse 1", "Clicks buttons and selects in process list."));
        assert_eq!(MENU_BANNERS[0], "┌─┐┌─┐┌┬┐┬┌─┐┌┐┌┌─┐");
    }
}
