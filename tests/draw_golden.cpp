// SPDX-License-Identifier: Apache-2.0
//
// tests/draw_golden.cpp — golden-output harness for the draw (M3) port.
//
// Dumps synthetic Meter/Graph/box outputs delimited by @@BEGIN:<name>@@ /
// @@END:<name>@@ markers. src-rust/fixtures/draw/capture.sh splits stdout
// into per-scenario <name>.ans fixtures and proves determinism (double run).
//
// Headless init WITHOUT Term::init(): Term::init() needs a real terminal
// (termios state + ioctl size queries) and fails outside a TTY. Instead we
// mirror the btop.cpp init sequence — Config defaults (btop.cpp:328 calls
// Config::load, but loading the host's ~/.config/btop/btop.conf would make
// output host-dependent, so we use the compiled-in defaults from
// btop_config.cpp:273-399 plus the fixed overrides in setup()) and
// Theme::updateThemes()/setTheme() (btop.cpp:1044-1045) — and set
// Term::width/height (btop_tools.hpp, Term namespace) directly.
//
// The scenario literals below are the single source of truth for the later
// Rust transcription tasks. All values are fixed; host-dependent sources
// (wall clock, uptime, battery, cpu freq/watts) are disabled via Config so
// output is stable run-to-run. Two known host-stable-but-host-specific
// inputs remain (documented, NOT normalized): Mem::get_totalMem() is called
// inside Mem::draw/Proc::draw (sysctl hw.memsize — stable on one machine).

#include <array>
#include <cstdio>
#include <deque>
#include <sstream>
#include <string>
#include <unordered_map>
#include <vector>

#include "btop_config.hpp"
#include "btop_draw.hpp"
#include "btop_menu.hpp"
#include "btop_shared.hpp"
#include "btop_theme.hpp"
#include "btop_tools.hpp"

namespace Cpu {
extern int b_columns, b_column_size;
extern int b_x, b_y, b_width, b_height;
}
namespace Mem {
extern int mem_width, disks_width, divider, item_height, mem_size, mem_meter;
extern int graph_height, disk_meter;
}
namespace Net {
extern int b_x, b_y, b_width, b_height, d_graph_height, u_graph_height;
}

namespace {

using std::array;
using std::deque;
using std::string;
using std::vector;
using namespace std::literals;

constexpr int S0_W = 100, S0_H = 30; // small terminal
constexpr int S1_W = 160, S1_H = 48; // large terminal

// (Geometry externs above are at global scope so they merge with the
// ::Cpu/::Mem/::Net definitions in src/btop_draw.cpp.)

void emit(const string& name, const string& out) {
	// NUL-safe: Proc detail header calls uresize(name, n, wide=true), whose
	// wide path resizes to wchar count and leaves wcstombs' terminator
	// embedded (btop_tools.cpp:269-288), so `out` can contain '\0'. printf
	// %s would truncate there; fwrite preserves the faithful bytes.
	printf("@@BEGIN:%s@@\n", name.c_str());
	fwrite(out.c_str(), 1, out.size(), stdout);
	printf("\n@@END:%s@@\n", name.c_str());
}

// Fixed 8-value graph dataset shared by the graph scenarios.
deque<long long> graph_data() {
	return {10, 20, 30, 40, 50, 60, 70, 80};
}

// Fixed 10-sample helper: {base, base+step, ...}.
deque<long long> ramp(long long base, long long step, int n = 10) {
	deque<long long> d;
	for (int i = 0; i < n; ++i) d.push_back(base + step * i);
	return d;
}

// Deterministic headless setup for one terminal size + box set.
// Replaces the terminal-dependent part of btop init (Term::init).
void setup(int w, int h, const string& boxes) {
	// Fixed terminal size, set directly (never Term::init() headless).
	Term::width.store(w);
	Term::height.store(h);

	// Determinism overrides: disable every host/time-dependent source.
	Config::set("clock_format", ""s); // wall clock (Draw::update_clock)
	Config::set("show_uptime", false); // system_uptime() in cpu box
	Config::set("show_battery", false); // battery sysfs/IOKit state
	Config::set("show_cpu_watts", false); // package power sensors
	Config::set("show_cpu_freq", false); // sysctl cpu freq string
	Config::set("cpu_graph_upper", "total"s);
	Config::set("cpu_graph_lower", "total"s);
	Config::set("proc_sorting", "pid"s);
	Config::set("proc_reversed", false);
	Config::set("proc_tree", false);
	Config::set("swap_disk", false); // show swap as meters, not a disk
	Config::set("io_mode", false);
	Config::set("show_gpu_info", "Off"s); // keep cpu box free of gpu state

	// Pinned machine description (normally filled by collect()).
	Shared::coreCount = 8;
	Cpu::cpuName = "Golden Test CPU 8-Core";
	Cpu::cpuHz.clear(); // NOTE (hasCpuHz trap): btop_draw.cpp:2366 declares
	// `static const bool hasCpuHz = not Cpu::get_cpuHz().empty()`, initialized
	// ONCE on the first Cpu::draw call. Clearing cpuHz before the first draw
	// pins hasCpuHz=false process-wide; any future scenario that populates
	// cpuHz AFTER the first draw would still silently observe false. Keep this
	// clear-first ordering (with show_cpu_freq=false above) for determinism.
	Cpu::available_fields = {"Auto", "total"};
	Cpu::got_sensors = true;
	Cpu::cpu_temp_only = false;
	Cpu::has_battery = false;
	Cpu::supports_watts = false;
	Cpu::current_bat = {0, 0.0f, 0, ""};
	Cpu::container_engine.reset();

	Mem::has_swap = true;
	Mem::disk_ios = 1; // one disk with io activity (matches fixture below)

	Net::interfaces = {"eth0"};
	Net::selected_iface = "eth0";
	Net::rescale = false;
	Net::graph_max = {{"download", 10000000}, {"upload", 5000000}};

	Proc::numpids = 3;
	Proc::selected_pid = 0;
	Proc::selected_name.clear();

	// "Default" + "TTY" only: theme dirs are unset headless, so no host
	// theme files are picked up. color_theme defaults to "Default".
	Theme::updateThemes();
	Theme::setTheme();

	// Gpu preconditions must hold before set_boxes("gpuN") validation
	// (Config::set_boxes requires Gpu::count > N).
	if (boxes.contains("gpu")) {
		Gpu::count = 1;
		Gpu::gpu_names = {"Golden GPU"};
		Gpu::gpu_b_height_offsets = {7};
	}

	Config::set_boxes(boxes);
	Config::set("shown_boxes", boxes);

	// Fresh per-pass state so each size pass is independent.
	Global::resized = false;
	Proc::resized = false;
	Proc::p_graphs.clear();
	Proc::p_counters.clear();

	Draw::calcSizes();
}

// Fixed cpu fixture: 20 total samples, 8 cores x 10, package + 8 core temps.
Cpu::cpu_info fixed_cpu() {
	Cpu::cpu_info cpu;
	cpu.cpu_percent["total"] = {12, 18, 25, 31, 27, 35, 42, 38, 45, 52,
								48, 55, 61, 58, 64, 70, 66, 72, 78, 75};
	cpu.core_percent = {
		{8, 12, 15, 18, 22, 25, 28, 30, 33, 35},
		{20, 25, 30, 35, 40, 45, 50, 55, 60, 65},
		{5, 10, 8, 12, 15, 10, 14, 18, 16, 20},
		{70, 65, 72, 68, 75, 71, 78, 74, 80, 77},
		{30, 32, 34, 36, 38, 40, 42, 44, 46, 48},
		{50, 48, 52, 55, 53, 57, 60, 58, 62, 65},
		{15, 18, 22, 20, 25, 28, 26, 30, 33, 31},
		{40, 45, 42, 48, 50, 47, 52, 55, 53, 58},
	};
	cpu.temp = {
		{55, 56, 55, 57, 56, 58, 57, 58, 59, 58}, // package
		{50, 51, 50, 52, 51, 53, 52, 53, 54, 53},
		{52, 52, 53, 53, 54, 54, 55, 55, 56, 56},
		{48, 49, 48, 50, 49, 51, 50, 52, 51, 53},
		{60, 61, 60, 62, 61, 63, 62, 64, 63, 65},
		{54, 55, 54, 56, 55, 57, 56, 58, 57, 59},
		{58, 58, 59, 59, 60, 60, 61, 61, 62, 62},
		{51, 52, 51, 53, 52, 54, 53, 55, 54, 56},
		{57, 57, 58, 58, 59, 59, 60, 60, 61, 61},
	};
	cpu.temp_max = 95;
	cpu.load_avg = {1.5, 1.2, 1.0};
	cpu.usage_watts = 0.0f;
	return cpu;
}

// Fixed mem fixture: single "/" disk (single-element containers keep the
// unordered_map iteration in Mem::draw stable run-to-run).
Mem::mem_info fixed_mem() {
	Mem::mem_info mem;
	mem.stats = {
		{"used", 8589934592}, // 8 GiB
		{"available", 3221225472},
		{"cached", 2147483648},
		{"free", 5368709120},
		{"swap_total", 4294967296}, // 4 GiB
		{"swap_used", 1073741824}, // 1 GiB
		{"swap_free", 3221225472},
	};
	mem.percent = {
		{"used", {62, 63, 64, 63, 65, 66, 65, 67, 68, 67}},
		{"available", {30, 31, 30, 32, 31, 33, 32, 34, 33, 35}},
		{"cached", {15, 15, 16, 16, 15, 17, 16, 18, 17, 18}},
		{"free", {25, 24, 25, 23, 24, 22, 23, 21, 22, 20}},
		{"swap_total", {25, 25, 25, 25, 25, 25, 25, 25, 25, 25}},
		{"swap_used", {20, 20, 21, 21, 22, 22, 23, 23, 24, 25}},
		{"swap_free", {80, 80, 79, 79, 78, 78, 77, 77, 76, 75}},
	};
	Mem::disk_info root;
	root.dev = "/dev/disk1s1";
	root.name = "/";
	root.total = 100000000000; // 100 GB
	root.used = 40000000000;
	root.free = 60000000000;
	root.used_percent = 40;
	root.free_percent = 60;
	root.io_read = ramp(1000000, 500000);
	root.io_write = ramp(500000, 250000);
	root.io_activity = {10, 15, 20, 25, 30, 35, 30, 25, 20, 15};
	mem.disks = {{"/", root}};
	mem.disks_order = {"/"};
	return mem;
}

// Fixed net fixture.
Net::net_info fixed_net() {
	Net::net_info net;
	net.bandwidth["download"] = {1000000, 1500000, 2000000, 2500000, 3000000,
								 3500000, 4000000, 4500000, 5000000, 4500000,
								 4000000, 3500000, 3000000, 2500000, 2000000,
								 2500000, 3000000, 3500000, 4000000, 4500000};
	net.bandwidth["upload"] = {500000, 600000, 700000, 800000, 900000,
							   1000000, 1100000, 1200000, 1300000, 1200000,
							   1100000, 1000000, 900000, 800000, 700000,
							   800000, 900000, 1000000, 1100000, 1200000};
	net.stat["download"] = {2500000, 8000000, 123456789012, 0, 0, 0};
	net.stat["upload"] = {1200000, 3000000, 45678901234, 0, 0, 0};
	net.ipv4 = "192.0.2.1";
	net.ipv6 = "";
	net.connected = true;
	return net;
}

// Fixed proc fixture: 3 processes, pid order.
vector<Proc::proc_info> fixed_procs() {
	vector<Proc::proc_info> plist;
	Proc::proc_info p1;
	p1.pid = 1;
	p1.name = "launchd";
	p1.cmd = "/sbin/launchd";
	p1.short_cmd = "launchd";
	p1.threads = 4;
	p1.user = "root";
	p1.mem = 12582912;
	p1.cpu_p = 0.5;
	p1.cpu_c = 0.5;
	p1.state = 'S';
	Proc::proc_info p2;
	p2.pid = 777;
	p2.name = "kernel_task";
	p2.cmd = "kernel_task";
	p2.short_cmd = "kernel_task";
	p2.threads = 128;
	p2.user = "root";
	p2.mem = 134217728;
	p2.cpu_p = 3.2;
	p2.cpu_c = 3.2;
	p2.state = 'S';
	Proc::proc_info p3;
	p3.pid = 4242;
	p3.name = "btop";
	p3.cmd = "btop --utf-force";
	p3.short_cmd = "btop";
	p3.threads = 3;
	p3.user = "tester";
	p3.mem = 67108864;
	p3.cpu_p = 12.5;
	p3.cpu_c = 12.5;
	p3.state = 'R';
	plist.push_back(p1);
	plist.push_back(p2);
	plist.push_back(p3);
	return plist;
}

// Fixed gpu fixture.
Gpu::gpu_info fixed_gpu() {
	Gpu::gpu_info gpu;
	gpu.gpu_percent["gpu-totals"] = {20, 25, 30, 35, 40, 45, 50, 55, 60, 55};
	gpu.gpu_percent["gpu-vram-totals"] = {40, 41, 42, 43, 44, 45, 46, 47, 48, 49};
	gpu.gpu_percent["gpu-pwr-totals"] = {50, 52, 54, 56, 58, 60, 58, 56, 54, 52};
	gpu.gpu_clock_speed = 1800; // MHz
	gpu.pwr_usage = 125000; // mW
	gpu.pwr_state = 8; // P-state (member has no default init; pin it — unpinned garbage drifted across rebuilds)
	gpu.temp = {55, 56, 57, 58, 59, 60};
	gpu.temp_max = 95;
	gpu.mem_total = 17179869184; // 16 GiB
	gpu.mem_used = 8589934592; // 8 GiB
	gpu.mem_utilization_percent = {45, 46, 47, 48, 49, 50, 51, 52, 53, 54};
	gpu.mem_clock_speed = 8000; // MHz
	gpu.pcie_tx = 102400; // KB/s
	gpu.pcie_rx = 204800;
	gpu.encoder_utilization = 25;
	gpu.decoder_utilization = 10;
	return gpu;
}

} // namespace

int main() {
	// Meter + Graph scenarios (need Theme gradients: setup() runs first).
	setup(S0_W, S0_H, "cpu mem net proc");
	emit("meter_50", Draw::Meter(50, "cpu", false)(75));
	emit("graph_default", Draw::Graph(20, 5, "cpu", graph_data(), "default")());
	emit("graph_tty", Draw::Graph(20, 5, "cpu", graph_data(), "tty")());

	// Box/banner/calcSizes fixtures. Still under the S0 setup above, so
	// the calcSizes dump reflects the S0 geometry. Stable key=value text
	// format (one line per box; also mirrored by the Rust layout_dump
	// test helper):
	//   "cpu x=.. y=.. w=.. h=.. bx=.. by=.. bw=.. bh=.. bcols=.. bcolsz=.."
	//   "mem x=.. y=.. w=.. h=.. memw=.. disksw=.. div=.. itemh=.. memsz=.. meterm=.. graphh=.. diskm=.."
	//   "net x=.. y=.. w=.. h=.. bx=.. by=.. bw=.. bh=.. dgraph=.. ugraph=.."
	//   "proc x=.. y=.. w=.. h=.. selmax=.."
	//   "gputotal h=.."
	emit("createBox", Draw::createBox(2, 3, 10, 5, "", false, "t", "b", 1));
	emit("banner_gen", Draw::banner_gen(2, 3, false, false));
	{
		std::ostringstream oss;
		oss << "cpu x=" << Cpu::x << " y=" << Cpu::y << " w=" << Cpu::width << " h=" << Cpu::height
			<< " bx=" << Cpu::b_x << " by=" << Cpu::b_y << " bw=" << Cpu::b_width << " bh=" << Cpu::b_height
			<< " bcols=" << Cpu::b_columns << " bcolsz=" << Cpu::b_column_size << "\n";
		oss << "mem x=" << Mem::x << " y=" << Mem::y << " w=" << Mem::width << " h=" << Mem::height
			<< " memw=" << Mem::mem_width << " disksw=" << Mem::disks_width << " div=" << Mem::divider
			<< " itemh=" << Mem::item_height << " memsz=" << Mem::mem_size << " meterm=" << Mem::mem_meter
			<< " graphh=" << Mem::graph_height << " diskm=" << Mem::disk_meter << "\n";
		oss << "net x=" << Net::x << " y=" << Net::y << " w=" << Net::width << " h=" << Net::height
			<< " bx=" << Net::b_x << " by=" << Net::b_y << " bw=" << Net::b_width << " bh=" << Net::b_height
			<< " dgraph=" << Net::d_graph_height << " ugraph=" << Net::u_graph_height << "\n";
		oss << "proc x=" << Proc::x << " y=" << Proc::y << " w=" << Proc::width << " h=" << Proc::height
			<< " selmax=" << Proc::select_max << "\n";
		oss << "gputotal h=" << Gpu::total_height;
		emit("calcSizes_S0", oss.str());
	}

	// Box scenarios at S0.
	{
		const auto cpu = fixed_cpu();
		const vector<Gpu::gpu_info> no_gpus;
		emit("cpu_S0", Cpu::draw(cpu, no_gpus, true, false));
	}
	{
		const auto mem = fixed_mem();
		emit("mem_S0", Mem::draw(mem, true, false));
	}
	{
		const auto net = fixed_net();
		emit("net_S0", Net::draw(net, true, false));
	}
	{
		const auto plist = fixed_procs();
		emit("proc_S0", Proc::draw(plist, true, false));
	}

	// Box scenarios at S1.
	setup(S1_W, S1_H, "cpu mem net proc");
	{
		const auto cpu = fixed_cpu();
		const vector<Gpu::gpu_info> no_gpus;
		emit("cpu_S1", Cpu::draw(cpu, no_gpus, true, false));
	}
	{
		const auto mem = fixed_mem();
		emit("mem_S1", Mem::draw(mem, true, false));
	}
	{
		const auto net = fixed_net();
		emit("net_S1", Net::draw(net, true, false));
	}
	{
		const auto plist = fixed_procs();
		emit("proc_S1", Proc::draw(plist, true, false));
	}

	// Gpu scenarios (own size pass: Gpu::draw indexes geometry sized by
	// calcSizes for shown gpu panels).
	setup(S0_W, S0_H, "gpu0");
	{
		const auto gpu = fixed_gpu();
		emit("gpu_S0", Gpu::draw(gpu, 0, true, false));
	}
	setup(S1_W, S1_H, "gpu0");
	{
		const auto gpu = fixed_gpu();
		emit("gpu_S1", Gpu::draw(gpu, 0, true, false));
	}

	// Proc detail/tree/filter scenarios at S0 (100x30, proc h=20 per
	// calcSizes_S0 — detail overhead is 8 rows, leaving a 12-row list, so
	// S0 suffices; tree/filter are list-only). Each does its own setup()
	// for an independent size pass. State is reset after each emit so the
	// menu block below still starts from defaults.
	{
		setup(S0_W, S0_H, "cpu mem net proc");
		Config::set("show_detailed", true);
		Config::set("detailed_pid", 4242);
		auto plist = fixed_procs();
		Proc::detailed = {};
		Proc::detailed.last_pid = 4242;
		Proc::detailed.entry = plist[2];
		Proc::detailed.status = "Running";
		Proc::detailed.elapsed = "12:34";
		Proc::detailed.parent = "launchd";
		Proc::detailed.io_read = "1.0M";
		Proc::detailed.io_write = "512K";
		Proc::detailed.memory = "64M";
		Proc::detailed.cpu_percent = {10, 20, 30, 40, 50, 60, 70, 80};
		Proc::detailed.mem_bytes = {67108864, 67108864, 67108864, 67108864,
									67108864, 67108864, 67108864, 67108864};
		Proc::detailed.first_mem = 134217728;
		Proc::numpids = 3;
		emit("proc_detail", Proc::draw(plist, true, false));
		Config::set("show_detailed", false);
		Config::set("detailed_pid", 0);
		Proc::detailed = {};
	}
	{
		setup(S0_W, S0_H, "cpu mem net proc");
		Config::set("proc_tree", true);
		auto plist = fixed_procs();
		plist[0].ppid = 0;
		plist[0].depth = 0;
		plist[0].prefix = "[-]\u2500";
		plist[0].tree_index = 0;
		plist[0].collapsed = false;
		plist[0].filtered = false;
		plist[1].ppid = 1;
		plist[1].depth = 1;
		plist[1].prefix = " \u251c\u2500";
		plist[1].tree_index = 1;
		plist[1].collapsed = false;
		plist[1].filtered = false;
		plist[2].ppid = 1;
		plist[2].depth = 1;
		plist[2].prefix = " \u2514\u2500";
		plist[2].tree_index = 2;
		plist[2].collapsed = false;
		plist[2].filtered = false;
		Proc::numpids = 3;
		emit("proc_tree", Proc::draw(plist, true, false));
		Config::set("proc_tree", false);
	}
	{
		setup(S0_W, S0_H, "cpu mem net proc");
		Config::set("proc_filter", "btop"s);
		Config::set("proc_filtering", false);
		auto plist = fixed_procs();
		plist[0].filtered = true;
		plist[1].filtered = true;
		plist[2].filtered = false;
		Proc::numpids = 1;
		Proc::filter_found = 2;
		emit("proc_filtered", Proc::draw(plist, true, false));
		Config::set("proc_filter", ""s);
		Proc::numpids = 3;
		Proc::filter_found = 0;
	}

	// Menu overlay scenarios at S0 (100x30, Default theme, same Config
	// overrides as setup()). Capture mechanics: Menu::show(menu) sets the
	// menuMask bit and calls Menu::process("") which synchronously runs the
	// menu body — the body populates Global::overlay when redraw is set.
	// No Runner::run("overlay") call is needed to POPULATE the string
	// (process() does call Runner::run("all", true, true) afterwards, which
	// is a harmless headless no-op: Runner::active is false so the waits
	// return immediately and thread_trigger() signals no thread).
	// Between scenarios the menu is closed via process("escape") so the
	// next show() starts fresh (menuMask/bg/currentMenu reset).
	setup(S0_W, S0_H, "cpu mem net proc");
	{
		Menu::show(Menu::Main);
		emit("menu_main", Global::overlay);
		Menu::process("escape");
	}
	{
		Menu::show(Menu::Options);
		emit("menu_options", Global::overlay);
		Menu::process("escape");
	}
	{
		Menu::show(Menu::Help);
		emit("menu_help", Global::overlay);
		Menu::process("escape");
	}
	{
		Menu::msgBox ok(45, Menu::msgBox::OK,
						{"Golden msgbox line one", "Golden msgbox line two"},
						"golden ok");
		emit("msgbox_ok", ok());
	}
	{
		Menu::msgBox yesno(45, Menu::msgBox::YES_NO,
						   {"Golden msgbox line one", "Golden msgbox line two"},
						   "golden yesno");
		emit("msgbox_yesno", yesno());
	}

	// Menu::process → Runner::run("all") leaves Config::locked == true even
	// headless. Unlock so any scenario appended after this block gets live
	// Config::set values instead of silent *Tmp staging.
	Config::unlock();

	return 0;
}
