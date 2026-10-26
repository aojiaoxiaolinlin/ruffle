// Standalone Windows diagnostic harness; built by Windows PowerShell 5.1 Add-Type.
using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Drawing;
using System.Drawing.Imaging;
using System.Globalization;
using System.IO;
using System.Linq;
using System.Runtime.InteropServices;
using System.Runtime.Serialization;
using System.Runtime.Serialization.Json;
using System.Security.Cryptography;
using System.Threading;
using System.Windows.Forms;

namespace MemorySession
{
    [DataContract]
    public sealed class InputEvent
    {
        [DataMember] public double Ms;
        [DataMember] public string Kind;
        [DataMember] public int X;
        [DataMember] public int Y;
        [DataMember] public int Code;
        [DataMember] public int Data;
        [DataMember] public bool Extended;
    }

    [DataContract]
    public sealed class Recording
    {
        [DataMember] public int Version = 1;
        [DataMember] public int Width;
        [DataMember] public int Height;
        [DataMember] public uint Dpi;
        [DataMember] public double DurationMs;
        [DataMember] public bool Complete;
        [DataMember] public List<InputEvent> Events = new List<InputEvent>();

        public void Validate()
        {
            if (Version != 1 || Width <= 0 || Height <= 0 || Dpi == 0 || !Complete ||
                !Finite(DurationMs) || DurationMs < 0 || Events == null || Events.Count > 1000000)
                throw new InvalidDataException("Unsupported or incomplete recording.");
            double previous = 0;
            var held = new HashSet<string>();
            foreach (InputEvent e in Events)
            {
                if (e == null || !Finite(e.Ms) || e.Ms < previous || e.Ms > DurationMs)
                    throw new InvalidDataException("Invalid event timeline.");
                previous = e.Ms;
                bool mouse = e.Kind == "move" || e.Kind == "down" || e.Kind == "up" || e.Kind == "wheel";
                bool key = e.Kind == "keyDown" || e.Kind == "keyUp";
                if (!mouse && !key && e.Kind != "checkpoint")
                    throw new InvalidDataException("Unknown event kind.");
                if (mouse && (e.X < 0 || e.Y < 0 || e.X >= Width || e.Y >= Height))
                    throw new InvalidDataException("Mouse event outside client area.");
                if ((e.Kind == "down" || e.Kind == "up") && (e.Code < 1 || e.Code > 3))
                    throw new InvalidDataException("Unsupported mouse button.");
                // F8/F9/F10 are harness controls. System/meta keys are deliberately unsupported.
                if (key && (e.Code <= 0 || e.Code > 254 || e.Code == 0x5b || e.Code == 0x5c ||
                    e.Code == 0x12 || e.Code == 0xa4 || e.Code == 0xa5 || (e.Code >= 0x77 && e.Code <= 0x79)))
                    throw new InvalidDataException("Unsupported recorded key.");
                string id = key ? "k" + e.Code : "m" + e.Code;
                if (e.Kind == "down" || e.Kind == "keyDown") held.Add(id);
                if ((e.Kind == "up" || e.Kind == "keyUp") && !held.Remove(id))
                    throw new InvalidDataException("Unbalanced input release.");
                if (e.Kind == "checkpoint" && held.Count != 0)
                    throw new InvalidDataException("Checkpoint contains held input.");
            }
            if (held.Count != 0) throw new InvalidDataException("Recording ends with held input.");
        }

        private static bool Finite(double x) { return !Double.IsNaN(x) && !Double.IsInfinity(x); }
    }

    internal static class Native
    {
        internal delegate IntPtr Hook(int code, IntPtr message, IntPtr data);
        [StructLayout(LayoutKind.Sequential)] internal struct Point { public int X, Y; }
        [StructLayout(LayoutKind.Sequential)] internal struct Rect { public int Left, Top, Right, Bottom; }
        [StructLayout(LayoutKind.Sequential)] internal struct MouseHook { public Point Point; public uint Data, Flags, Time; public UIntPtr Extra; }
        [StructLayout(LayoutKind.Sequential)] internal struct KeyHook { public uint Vk, Scan, Flags, Time; public UIntPtr Extra; }
        [StructLayout(LayoutKind.Sequential)] internal struct MouseInput { public int X, Y; public uint Data, Flags, Time; public UIntPtr Extra; }
        [StructLayout(LayoutKind.Sequential)] internal struct KeyInput { public ushort Vk, Scan; public uint Flags, Time; public UIntPtr Extra; }
        [StructLayout(LayoutKind.Explicit)] internal struct InputUnion
        {
            [FieldOffset(0)] public MouseInput Mouse;
            [FieldOffset(0)] public KeyInput Key;
        }
        [StructLayout(LayoutKind.Sequential)] internal struct Input { public uint Type; public InputUnion Value; }
        [StructLayout(LayoutKind.Sequential)] internal struct MemoryCounters
        {
            public uint Size, PageFaultCount;
            public UIntPtr PeakWorkingSetSize, WorkingSetSize, QuotaPeakPagedPoolUsage, QuotaPagedPoolUsage;
            public UIntPtr QuotaPeakNonPagedPoolUsage, QuotaNonPagedPoolUsage, PagefileUsage, PeakPagefileUsage, PrivateUsage;
        }
        [DllImport("user32.dll")] internal static extern IntPtr SetThreadDpiAwarenessContext(IntPtr context);
        [DllImport("user32.dll")] internal static extern IntPtr GetForegroundWindow();
        [DllImport("user32.dll")] internal static extern uint GetWindowThreadProcessId(IntPtr window, out uint pid);
        [DllImport("user32.dll")] internal static extern bool GetClientRect(IntPtr window, out Rect rect);
        [DllImport("user32.dll")] internal static extern bool ClientToScreen(IntPtr window, ref Point point);
        [DllImport("user32.dll")] internal static extern bool ScreenToClient(IntPtr window, ref Point point);
        [DllImport("user32.dll")] internal static extern bool IsWindow(IntPtr window);
        [DllImport("user32.dll")] internal static extern bool IsIconic(IntPtr window);
        [DllImport("user32.dll")] internal static extern uint GetDpiForWindow(IntPtr window);
        [DllImport("user32.dll")] internal static extern short GetAsyncKeyState(int key);
        [DllImport("user32.dll")] internal static extern int GetSystemMetrics(int index);
        [DllImport("user32.dll", SetLastError = true)] internal static extern uint SendInput(uint count, Input[] inputs, int size);
        [DllImport("user32.dll", SetLastError = true)] internal static extern IntPtr SetWindowsHookEx(int type, Hook hook, IntPtr module, uint thread);
        [DllImport("user32.dll")] internal static extern bool UnhookWindowsHookEx(IntPtr hook);
        [DllImport("user32.dll")] internal static extern IntPtr CallNextHookEx(IntPtr hook, int code, IntPtr message, IntPtr data);
        [DllImport("kernel32.dll", CharSet = CharSet.Unicode)] internal static extern IntPtr GetModuleHandle(string name);
        [DllImport("psapi.dll", SetLastError = true)] internal static extern bool GetProcessMemoryInfo(IntPtr process, ref MemoryCounters counters, uint size);
        [DllImport("pdh.dll", CharSet = CharSet.Unicode)] internal static extern uint PdhOpenQuery(string source, UIntPtr data, out IntPtr query);
        [DllImport("pdh.dll", CharSet = CharSet.Unicode)] internal static extern uint PdhAddEnglishCounter(IntPtr query, string path, UIntPtr data, out IntPtr counter);
        [DllImport("pdh.dll")] internal static extern uint PdhCollectQueryData(IntPtr query);
        [DllImport("pdh.dll")] internal static extern uint PdhCloseQuery(IntPtr query);
        [DllImport("pdh.dll", CharSet = CharSet.Unicode, EntryPoint = "PdhGetFormattedCounterArrayW")]
        internal static extern uint PdhArray(IntPtr counter, uint format, ref uint bytes, out uint count, IntPtr buffer);
        [StructLayout(LayoutKind.Explicit, Size = 16)] internal struct CounterValue
        {
            [FieldOffset(0)] public uint Status;
            [FieldOffset(8)] public long Value;
        }
        [StructLayout(LayoutKind.Sequential)] internal struct CounterItem { public IntPtr Name; public CounterValue Value; }
    }

    // PDH English paths also work on localized Windows. Unavailable counters stay blank.
    internal sealed class Counters : IDisposable
    {
        private IntPtr query;
        private readonly Dictionary<string, IntPtr> handles = new Dictionary<string, IntPtr>();
        internal Counters()
        {
            if (Native.PdhOpenQuery(null, UIntPtr.Zero, out query) != 0) return;
            Add("pid", @"\Process(*)\ID Process");
            Add("private_ws", @"\Process(*)\Working Set - Private");
            Add("gpu_dedicated", @"\GPU Process Memory(*)\Dedicated Usage");
            Add("gpu_shared", @"\GPU Process Memory(*)\Shared Usage");
        }
        private void Add(string name, string path)
        {
            IntPtr counter;
            if (Native.PdhAddEnglishCounter(query, path, UIntPtr.Zero, out counter) == 0) handles.Add(name, counter);
        }
        internal bool Collect() { return query != IntPtr.Zero && Native.PdhCollectQueryData(query) == 0; }
        internal Dictionary<string, long> Read(string name)
        {
            var result = new Dictionary<string, long>(StringComparer.OrdinalIgnoreCase);
            IntPtr counter;
            if (!handles.TryGetValue(name, out counter)) return result;
            uint bytes = 0, count;
            if (Native.PdhArray(counter, 0x400, ref bytes, out count, IntPtr.Zero) != 0x800007d2 || bytes == 0) return result;
            IntPtr buffer = Marshal.AllocHGlobal(checked((int)bytes));
            try
            {
                if (Native.PdhArray(counter, 0x400, ref bytes, out count, buffer) != 0) return result;
                int size = Marshal.SizeOf(typeof(Native.CounterItem));
                for (int i = 0; i < count; i++)
                {
                    var item = (Native.CounterItem)Marshal.PtrToStructure(IntPtr.Add(buffer, i * size), typeof(Native.CounterItem));
                    if (item.Value.Status <= 1) result[Marshal.PtrToStringUni(item.Name)] = item.Value.Value;
                }
            }
            finally { Marshal.FreeHGlobal(buffer); }
            return result;
        }
        internal static long? GpuForPid(Dictionary<string, long> values, int pid)
        {
            var matches = values.Where(x => x.Key.StartsWith("pid_" + pid + "_", StringComparison.OrdinalIgnoreCase)).ToArray();
            return matches.Length == 0 ? (long?)null : matches.Sum(x => x.Value);
        }
        public void Dispose() { if (query != IntPtr.Zero) Native.PdhCloseQuery(query); }
    }

    internal sealed class Sampler : IDisposable
    {
        private readonly ManualResetEvent stop = new ManualResetEvent(false);
        private readonly Thread thread;
        private readonly Session session;
        internal Sampler(Session owner, int pid, string output)
        {
            session = owner;
            thread = new Thread(() => Sample(pid, output));
            thread.IsBackground = true;
            thread.Start();
        }
        private void Sample(int pid, string output)
        {
            try
            {
                using (var process = Process.GetProcessById(pid))
                using (var counters = new Counters())
                using (var writer = new StreamWriter(Path.Combine(output, "memory.csv")))
                {
                    writer.WriteLine("utc,session_ms,route_ms,phase,checkpoint,pid,private_commit_bytes,working_set_bytes,private_working_set_bytes,gpu_dedicated_bytes,gpu_shared_bytes,sample_duration_ms");
                    do
                    {
                        if (process.HasExited) break;
                        var duration = Stopwatch.StartNew();
                        string stamp = DateTime.UtcNow.ToString("o");
                        string state = session.SampleState();
                        var memory = new Native.MemoryCounters();
                        memory.Size = (uint)Marshal.SizeOf(typeof(Native.MemoryCounters));
                        bool ok = Native.GetProcessMemoryInfo(process.Handle, ref memory, memory.Size);
                        long? privateWs = null, dedicated = null, shared = null;
                        if (counters.Collect())
                        {
                            var ids = counters.Read("pid");
                            var ws = counters.Read("private_ws");
                            foreach (var entry in ids)
                            {
                                long value;
                                if (entry.Value == pid && ws.TryGetValue(entry.Key, out value)) privateWs = value;
                            }
                            dedicated = Counters.GpuForPid(counters.Read("gpu_dedicated"), pid);
                            shared = Counters.GpuForPid(counters.Read("gpu_shared"), pid);
                        }
                        writer.WriteLine(String.Join(",", stamp, state, pid.ToString(CultureInfo.InvariantCulture),
                            ok ? memory.PrivateUsage.ToUInt64().ToString(CultureInfo.InvariantCulture) : "",
                            ok ? memory.WorkingSetSize.ToUInt64().ToString(CultureInfo.InvariantCulture) : "",
                            Number(privateWs), Number(dedicated), Number(shared),
                            duration.Elapsed.TotalMilliseconds.ToString("F1", CultureInfo.InvariantCulture)));
                        writer.Flush();
                    } while (!stop.WaitOne(1000));
                }
            }
            catch (Exception e) { session.SamplerError = e.Message; }
        }
        private static string Number(long? value) { return value.HasValue ? value.Value.ToString(CultureInfo.InvariantCulture) : ""; }
        public void Dispose() { stop.Set(); thread.Join(); stop.Dispose(); }
    }

    public sealed class Session : ApplicationContext
    {
        private readonly int pid;
        private readonly bool replay, unattended, observe;
        private readonly string output;
        private readonly Recording recording;
        private readonly Stopwatch sessionClock = Stopwatch.StartNew();
        private readonly Stopwatch routeClock = new Stopwatch();
        private readonly System.Windows.Forms.Timer timer = new System.Windows.Forms.Timer();
        private readonly StreamWriter timeline;
        private readonly HashSet<int> heldKeys = new HashSet<int>();
        private readonly HashSet<int> extendedKeys = new HashSet<int>();
        private readonly HashSet<int> heldButtons = new HashSet<int>();
        private readonly HashSet<int> controls = new HashSet<int>();
        private readonly object stateLock = new object();
        private Native.Hook mouseCallback, keyCallback;
        private IntPtr mouseHook, keyHook, window;
        private string phase = "armed";
        private int checkpoint, nextEvent;
        private bool done, started, complete;
        private string outcome = "stopped";
        private double maxLatenessMs;
        private Sampler sampler;
        public volatile string SamplerError;

        internal Session(int processId, string mode, string path, string directory, bool automatic)
        {
            pid = processId;
            replay = mode == "replay";
            observe = mode == "observe";
            unattended = automatic;
            output = directory;
            recording = replay ? ReadRecording(path) : new Recording();
            if (replay) recording.Validate();
            using (var process = Process.GetProcessById(pid))
            {
                string executable = process.MainModule.FileName;
                string hash;
                using (var sha = SHA256.Create())
                using (var file = File.OpenRead(executable)) hash = BitConverter.ToString(sha.ComputeHash(file)).Replace("-", "").ToLowerInvariant();
                File.WriteAllText(Path.Combine(output, "build.txt"), "pid=" + pid + "\r\nexe=" + executable +
                    "\r\nsha256=" + hash + "\r\nmode=" + mode + "\r\nunattended=" + unattended +
                    "\r\nstarted_utc=" + DateTime.UtcNow.ToString("o") + "\r\n");
            }
            timeline = new StreamWriter(Path.Combine(output, "timeline.csv"));
            timeline.WriteLine("utc,session_ms,route_ms,phase,checkpoint,event,lateness_ms");
            if (replay) Save(Path.Combine(output, "recording.json"), recording);
            timer.Interval = 5;
            timer.Tick += Tick;
        }

        public static void Run(int processId, string mode, string recordingPath, string directory, bool unattended)
        {
            if (IntPtr.Size != 8) throw new InvalidOperationException("Use 64-bit Windows PowerShell.");
            if (Native.SetThreadDpiAwarenessContext(new IntPtr(-4)) == IntPtr.Zero)
                throw new InvalidOperationException("Cannot enable physical-pixel coordinates. Windows 10 1703 or newer is required.");
            using (var session = new Session(processId, mode, recordingPath, directory, unattended))
                session.Execute();
        }

        internal void Execute()
        {
            try
            {
                InstallHooks();
                sampler = new Sampler(this, pid, output);
                timer.Start();
                Console.WriteLine(observe
                    ? "Observe: F8=start idle, F9=next stage (maps/battles/cooldown), F7=battle ended, F10=finish."
                    : "Focus the game after login. F8=start/resume, F9=checkpoint, F10=finish/stop.");
                Console.WriteLine("Output: " + output);
                if (observe) Console.WriteLine("Switching apps keeps sampling; background samples are marked separately. Return to the game for F9/F7; F10 finishes anywhere.");
                Application.Run(this);
            }
            finally { Finish(); }
        }

        private void InstallHooks()
        {
            mouseCallback = Mouse;
            keyCallback = Key;
            IntPtr module = Native.GetModuleHandle(null);
            if (!observe) mouseHook = Native.SetWindowsHookEx(14, mouseCallback, module, 0);
            keyHook = Native.SetWindowsHookEx(13, keyCallback, module, 0);
            if ((!observe && mouseHook == IntPtr.Zero) || keyHook == IntPtr.Zero)
                throw new System.ComponentModel.Win32Exception(Marshal.GetLastWin32Error());
        }

        internal string SampleState()
        {
            lock (stateLock)
                return String.Join(",", sessionClock.Elapsed.TotalMilliseconds.ToString("F1", CultureInfo.InvariantCulture),
                    routeClock.Elapsed.TotalMilliseconds.ToString("F1", CultureInfo.InvariantCulture), phase, checkpoint);
        }
        private void Log(string kind, double late)
        {
            timeline.WriteLine(DateTime.UtcNow.ToString("o") + "," + SampleState() + "," + kind + "," + late.ToString("F1", CultureInfo.InvariantCulture));
            timeline.Flush();
        }
        private bool ForegroundMatches()
        {
            uint current;
            IntPtr foreground = Native.GetForegroundWindow();
            Native.GetWindowThreadProcessId(foreground, out current);
            return current == pid && (!started || foreground == window);
        }
        private bool GeometryMatches()
        {
            Native.Rect rect;
            return Native.IsWindow(window) && !Native.IsIconic(window) && Native.GetClientRect(window, out rect) &&
                rect.Right == recording.Width && rect.Bottom == recording.Height && Native.GetDpiForWindow(window) == recording.Dpi;
        }
        private bool AnyHeld() { return heldKeys.Count != 0 || heldButtons.Count != 0; }
        internal void StartOrResume()
        {
            if (!ForegroundMatches()) { Console.WriteLine("Focus the target game window first."); return; }
            if (!started)
            {
                // Do not start halfway through a drag, modifier chord, or key press.
                for (int key = 1; key < 255; key++)
                    if (!(observe && key == 0x76) && key != 0x77 && key != 0x78 && key != 0x79 && (Native.GetAsyncKeyState(key) & 0x8000) != 0)
                    { Console.WriteLine("Release all mouse buttons and keys, then press F8."); return; }
                window = Native.GetForegroundWindow();
                Native.Rect rect;
                if (!Native.GetClientRect(window, out rect)) throw new InvalidOperationException("Cannot read game client size.");
                if (!replay)
                {
                    recording.Width = rect.Right;
                    recording.Height = rect.Bottom;
                    recording.Dpi = Native.GetDpiForWindow(window);
                }
                if (!GeometryMatches()) { Console.WriteLine("Match the recording's client size and display DPI before F8: " + recording.Width + "x" + recording.Height + " @ " + recording.Dpi); return; }
                started = true;
                Screenshot("start");
            }
            if (phase != "armed" && phase != "checkpoint") return;
            if (!GeometryMatches()) { Stop("window_geometry_changed", false); return; }
            lock (stateLock) { phase = "running"; routeClock.Start(); }
            Log("resume", 0);
            Console.WriteLine("Running.");
        }

        internal static string ObservationStage(int index)
        {
            switch (index)
            {
                case 0: return "idle";
                case 1: return "maps";
                case 2: return "battles";
                case 3: return "cooldown";
                default: throw new ArgumentOutOfRangeException("index");
            }
        }

        private void AdvanceObservation()
        {
            if (phase != "running" || checkpoint >= 3) return;
            lock (stateLock) { checkpoint++; }
            Log("stage_" + ObservationStage(checkpoint), 0);
            // No pause or screenshot I/O during continuous gameplay.
            Console.WriteLine("Stage: " + ObservationStage(checkpoint));
        }

        private void Checkpoint()
        {
            if (phase != "running" || AnyHeld()) return;
            if (!replay) Add(new InputEvent { Kind = "checkpoint" });
            lock (stateLock) { routeClock.Stop(); checkpoint++; phase = "checkpoint"; }
            Log("checkpoint", 0);
            Screenshot("checkpoint-" + checkpoint.ToString("D3"));
            if (replay && unattended) StartOrResume();
            else Console.WriteLine("Checkpoint " + checkpoint + ": verify the scene, then F8 to continue.");
        }

        private void Screenshot(string name)
        {
            Native.Point origin = new Native.Point();
            if (!Native.ClientToScreen(window, ref origin)) throw new InvalidOperationException("Cannot locate window.");
            using (var bitmap = new Bitmap(recording.Width, recording.Height))
            using (var graphics = Graphics.FromImage(bitmap))
            {
                graphics.CopyFromScreen(origin.X, origin.Y, 0, 0, bitmap.Size);
                bitmap.Save(Path.Combine(output, name + ".png"), ImageFormat.Png);
            }
        }

        private void Add(InputEvent e)
        {
            if (recording.Events.Count >= 1000000) throw new InvalidOperationException("Recording event limit reached.");
            e.Ms = routeClock.Elapsed.TotalMilliseconds;
            recording.Events.Add(e);
        }

        private IntPtr Mouse(int code, IntPtr message, IntPtr data)
        {
            try
            {
                if (code >= 0 && !done && started && phase == "running" && ForegroundMatches())
                {
                    var input = (Native.MouseHook)Marshal.PtrToStructure(data, typeof(Native.MouseHook));
                    bool injected = (input.Flags & 1) != 0;
                    if (replay && !injected)
                    {
                        // Keep physical input out of a replay; F10 always stops it.
                        return new IntPtr(1);
                    }
                    if (!replay && !injected)
                    {
                        Native.Point p = input.Point;
                        Native.ScreenToClient(window, ref p);
                        if (p.X < 0 || p.Y < 0 || p.X >= recording.Width || p.Y >= recording.Height)
                        { Stop("pointer_left_client", false); return Native.CallNextHookEx(IntPtr.Zero, code, message, data); }
                        int m = message.ToInt32();
                        var e = new InputEvent { X = p.X, Y = p.Y };
                        if (m == 0x200) e.Kind = "move";
                        else if (m == 0x201 || m == 0x204 || m == 0x207) { e.Kind = "down"; e.Code = m == 0x201 ? 1 : m == 0x204 ? 2 : 3; heldButtons.Add(e.Code); }
                        else if (m == 0x202 || m == 0x205 || m == 0x208) { e.Kind = "up"; e.Code = m == 0x202 ? 1 : m == 0x205 ? 2 : 3; if (!heldButtons.Remove(e.Code)) return Native.CallNextHookEx(IntPtr.Zero, code, message, data); }
                        else if (m == 0x20a || m == 0x20e) { e.Kind = "wheel"; e.Data = (short)(input.Data >> 16); e.Extended = m == 0x20e; }
                        else { Stop("unsupported_mouse_input", false); return Native.CallNextHookEx(IntPtr.Zero, code, message, data); }
                        Add(e);
                    }
                }
            }
            catch (Exception e) { Fail(e); }
            return Native.CallNextHookEx(IntPtr.Zero, code, message, data);
        }

        private IntPtr Key(int code, IntPtr message, IntPtr data)
        {
            try
            {
                if (code >= 0)
                {
                    var input = (Native.KeyHook)Marshal.PtrToStructure(data, typeof(Native.KeyHook));
                    bool injected = (input.Flags & 0x10) != 0;
                    bool up = (input.Flags & 0x80) != 0;
                    int key = (int)input.Vk;
                    if (!injected && key >= (observe ? 0x76 : 0x77) && key <= 0x79 && (ForegroundMatches() || key == 0x79 || controls.Contains(key)))
                    {
                        if (up) controls.Remove(key);
                        else if (controls.Add(key))
                        {
                            // Defer disk I/O/screenshots out of the low-level hook.
                            pendingControl = key;
                        }
                        return new IntPtr(1);
                    }
                    if (!observe && !done && started && phase == "running" && ForegroundMatches() && !injected)
                    {
                        if (replay) return new IntPtr(1);
                        if (key == 0x5b || key == 0x5c || key == 0x12 || key == 0xa4 || key == 0xa5)
                        { Stop("unsupported_system_key", false); return Native.CallNextHookEx(IntPtr.Zero, code, message, data); }
                        if (up && !heldKeys.Remove(key)) return Native.CallNextHookEx(IntPtr.Zero, code, message, data);
                        if (!up) heldKeys.Add(key);
                        Add(new InputEvent { Kind = up ? "keyUp" : "keyDown", Code = key, Data = (int)input.Scan, Extended = (input.Flags & 1) != 0 });
                    }
                }
            }
            catch (Exception e) { Fail(e); }
            return Native.CallNextHookEx(IntPtr.Zero, code, message, data);
        }

        private int pendingControl;
        private void Tick(object sender, EventArgs args)
        {
            try
            {
                if (done) { ExitThread(); return; }
                if (SamplerError != null) throw new InvalidOperationException("Memory sampling failed: " + SamplerError);
                int command = pendingControl;
                pendingControl = 0;
                if (command == 0x79)
                {
                    bool success = !replay && started && !AnyHeld() &&
                        (observe || (ForegroundMatches() && GeometryMatches()));
                    if (success)
                    {
                        routeClock.Stop();
                        if (ForegroundMatches() && GeometryMatches()) Screenshot("end");
                    }
                    Stop(replay ? "user_stopped_replay" : observe ? "observation_finished" : "recording_finished", success);
                    return;
                }
                if (command == 0x77) StartOrResume();
                if (command == 0x78 && !replay)
                {
                    if (observe) AdvanceObservation(); else Checkpoint();
                }
                if (command == 0x76 && observe && phase == "running" && checkpoint == 2)
                    Log("battle_end", 0);
                if (!started) return;
                if (!Native.IsWindow(window)) { Stop("window_closed", false); return; }
                if (observe && !done)
                {
                    bool foreground = ForegroundMatches();
                    if (!foreground && phase == "running")
                    {
                        lock (stateLock) { phase = "background"; }
                        Log("focus_lost", 0);
                        Console.WriteLine("Game in background; sampling continues.");
                    }
                    else if (foreground && phase == "background")
                    {
                        lock (stateLock) { phase = "running"; }
                        Log("focus_restored", 0);
                        Console.WriteLine("Game in foreground; sampling continues.");
                    }
                    if (!foreground) return;
                }
                if (phase != "running" || done) return;
                if (!ForegroundMatches()) { Stop("focus_lost", false); return; }
                if (!GeometryMatches()) { Stop("window_geometry_changed", false); return; }
                if (!replay) return;
                // Preserve ordering but cap catch-up work; excessive delay invalidates the run.
                int dispatched = 0;
                while (nextEvent < recording.Events.Count && recording.Events[nextEvent].Ms <= routeClock.Elapsed.TotalMilliseconds && dispatched++ < 128)
                {
                    InputEvent e = recording.Events[nextEvent++];
                    double late = routeClock.Elapsed.TotalMilliseconds - e.Ms;
                    maxLatenessMs = Math.Max(maxLatenessMs, late);
                    if (late > 250) { Stop("replay_more_than_250ms_late", false); return; }
                    if (e.Kind == "checkpoint") { Checkpoint(); return; }
                    Inject(e);
                    if (done) return;
                    if (e.Kind != "move") Log("replay_" + e.Kind, late);
                }
                if (nextEvent == recording.Events.Count && routeClock.Elapsed.TotalMilliseconds >= recording.DurationMs)
                    { routeClock.Stop(); Screenshot("end"); Stop("replay_finished", true); }
            }
            catch (Exception e) { Fail(e); }
        }

        private void Inject(InputEvent e)
        {
            if (!ForegroundMatches() || !GeometryMatches()) throw new InvalidOperationException("Target window changed before input dispatch.");
            if (e.Kind == "keyDown" || e.Kind == "keyUp")
            {
                SendKey(e.Code, e.Kind == "keyUp", e.Extended);
                if (e.Kind == "keyDown")
                {
                    heldKeys.Add(e.Code);
                    if (e.Extended) extendedKeys.Add(e.Code);
                }
                else { heldKeys.Remove(e.Code); extendedKeys.Remove(e.Code); }
                return;
            }
            Native.Point point = new Native.Point { X = e.X, Y = e.Y };
            Native.ClientToScreen(window, ref point);
            int left = Native.GetSystemMetrics(76), top = Native.GetSystemMetrics(77);
            int width = Native.GetSystemMetrics(78), height = Native.GetSystemMetrics(79);
            var move = new Native.Input { Type = 0 };
            move.Value.Mouse = new Native.MouseInput {
                X = Absolute(point.X, left, width), Y = Absolute(point.Y, top, height),
                Flags = 0x8000 | 0x4000 | 0x0001 | 0x2000
            };
            Send(move);
            if (e.Kind == "move") return;
            var action = new Native.Input { Type = 0 };
            if (e.Kind == "wheel")
            {
                action.Value.Mouse.Flags = e.Extended ? 0x1000u : 0x0800u;
                action.Value.Mouse.Data = unchecked((uint)e.Data);
            }
            else
            {
                bool up = e.Kind == "up";
                action.Value.Mouse.Flags = MouseFlag(e.Code, up);
            }
            Send(action);
            if (e.Kind == "down") heldButtons.Add(e.Code);
            if (e.Kind == "up") heldButtons.Remove(e.Code);
        }
        internal static int Absolute(int value, int origin, int extent)
        {
            if (extent <= 0 || value < origin || value >= (long)origin + extent)
                throw new InvalidOperationException("Recorded point is outside the virtual desktop.");
            return (int)Math.Min(65535, ((long)(value - origin) * 65536 + 32768) / extent);
        }
        private static uint MouseFlag(int code, bool up) { return code == 1 ? (up ? 4u : 2u) : code == 2 ? (up ? 16u : 8u) : (up ? 64u : 32u); }
        private static void SendKey(int key, bool up, bool extended)
        {
            var input = new Native.Input { Type = 1 };
            input.Value.Key = new Native.KeyInput { Vk = (ushort)key, Flags = (up ? 2u : 0u) | (extended ? 1u : 0u) };
            Send(input);
        }
        private static void Send(Native.Input input)
        {
            if (Native.SendInput(1, new[] { input }, Marshal.SizeOf(typeof(Native.Input))) != 1)
                throw new InvalidOperationException("SendInput failed. Run the game and harness at the same privilege level.");
        }

        private void Stop(string reason, bool success)
        {
            if (done) return;
            lock (stateLock) { routeClock.Stop(); phase = success ? "complete" : "invalid"; }
            outcome = reason;
            complete = success;
            done = true;
        }
        private void Fail(Exception error) { Console.WriteLine(error.Message); Stop("error", false); }
        private void Finish()
        {
            timer.Stop();
            timer.Dispose();
            if (mouseHook != IntPtr.Zero) Native.UnhookWindowsHookEx(mouseHook);
            if (keyHook != IntPtr.Zero) Native.UnhookWindowsHookEx(keyHook);
            // Pair injected downs with ups even after an abort; never release unrelated inputs.
            if (replay)
            {
                foreach (int key in heldKeys) { try { SendKey(key, true, extendedKeys.Contains(key)); } catch { } }
                foreach (int button in heldButtons)
                {
                    var input = new Native.Input { Type = 0 };
                    input.Value.Mouse.Flags = MouseFlag(button, true);
                    try { Send(input); } catch { }
                }
            }
            if (sampler != null) sampler.Dispose();
            if (SamplerError != null) { complete = false; outcome = "sampler_error"; }
            if (!replay)
            {
                recording.DurationMs = routeClock.Elapsed.TotalMilliseconds;
                recording.Complete = complete;
                if (complete && !observe)
                {
                    try { recording.Validate(); }
                    catch (InvalidDataException) { recording.Complete = complete = false; outcome = "invalid_recording"; }
                }
                if (observe) recording.Complete = false; // Observations contain no replayable input.
                Save(Path.Combine(output, observe ? "session.json" : "recording.json"), recording);
            }
            Log(outcome, maxLatenessMs);
            timeline.Dispose();
            File.WriteAllText(Path.Combine(output, "result.txt"), "complete=" + complete + "\r\noutcome=" + outcome +
                "\r\nmax_dispatch_lateness_ms=" + maxLatenessMs.ToString("F1", CultureInfo.InvariantCulture) +
                "\r\nscene_validation=" + (replay ? (unattended ? "unreviewed" : "manual_checkpoints") : observe ? "manual_observation" : "recorded") +
                "\r\nsampler_error=" + (SamplerError ?? "") + "\r\n");
            Console.WriteLine("Session ended: " + outcome + ". Output: " + output);
        }
        private static Recording ReadRecording(string path)
        {
            if (new FileInfo(path).Length > 256 * 1024 * 1024) throw new InvalidDataException("Recording too large.");
            using (var file = File.OpenRead(path))
                return (Recording)new DataContractJsonSerializer(typeof(Recording)).ReadObject(file);
        }
        internal static void Save(string path, Recording value)
        {
            using (var file = File.Create(path)) new DataContractJsonSerializer(typeof(Recording)).WriteObject(file, value);
        }
    }

    public static class SelfTest
    {
        public static void Run()
        {
            if (Session.ObservationStage(0) != "idle" || Session.ObservationStage(2) != "battles" || Session.ObservationStage(3) != "cooldown")
                throw new Exception("Observation stage order mismatch.");
            if (Marshal.SizeOf(typeof(Native.Input)) != (IntPtr.Size == 8 ? 40 : 28)) throw new Exception("INPUT ABI mismatch.");
            if (IntPtr.Size != 8 || Marshal.SizeOf(typeof(Native.CounterItem)) != 24) throw new Exception("Use 64-bit PowerShell (PDH ABI).");
            var recording = new Recording { Width = 800, Height = 600, Dpi = 96, DurationMs = 100, Complete = true };
            recording.Events.Add(new InputEvent { Kind = "down", Code = 1, X = 10, Y = 20, Ms = 10 });
            recording.Events.Add(new InputEvent { Kind = "up", Code = 1, X = 10, Y = 20, Ms = 20 });
            recording.Events.Add(new InputEvent { Kind = "checkpoint", Ms = 90 });
            recording.Validate();
            recording.Events[1].Ms = 5;
            ExpectInvalid(recording);
            recording.Events[1].Ms = 20;
            recording.Events[1].Kind = "move";
            ExpectInvalid(recording);
            recording.Events[1].Kind = "up";
            recording.Events[0].X = 800;
            ExpectInvalid(recording);
            recording.Events[0].X = 10;
            recording.Complete = false;
            ExpectInvalid(recording);
            if (Session.Absolute(-1920, -1920, 3840) < 0 || Session.Absolute(1919, -1920, 3840) > 65535) throw new Exception("Virtual desktop conversion failed.");
            var values = new Dictionary<string, long> { { "pid_12_luid_a", 10 }, { "pid_12_luid_b", 20 }, { "pid_123_luid_a", 999 } };
            if (Counters.GpuForPid(values, 12) != 30 || Counters.GpuForPid(values, 9).HasValue) throw new Exception("GPU PID aggregation failed.");
            // Exercise live OS metrics without installing hooks or injecting any input.
            using (var process = Process.GetCurrentProcess())
            using (var counters = new Counters())
            {
                var memory = new Native.MemoryCounters { Size = (uint)Marshal.SizeOf(typeof(Native.MemoryCounters)) };
                if (!Native.GetProcessMemoryInfo(process.Handle, ref memory, memory.Size) || memory.PrivateUsage.ToUInt64() == 0) throw new Exception("Process memory query failed.");
                if (!counters.Collect() || !counters.Read("pid").Values.Contains(process.Id)) throw new Exception("PDH PID query failed.");
                var ids = counters.Read("pid");
                var ws = counters.Read("private_ws");
                if (!ids.Any(x => x.Value == process.Id && ws.ContainsKey(x.Key))) throw new Exception("Private working set query failed.");
            }
        }
        private static void ExpectInvalid(Recording recording)
        {
            try { recording.Validate(); } catch (InvalidDataException) { return; }
            throw new Exception("Invalid recording accepted.");
        }

        // Explicit opt-in smoke test targets only its own temporary window.
        public static void ReplaySmoke(string directory)
        {
            if (Native.SetThreadDpiAwarenessContext(new IntPtr(-4)) == IntPtr.Zero)
                throw new InvalidOperationException("Cannot enable physical-pixel coordinates.");
            Directory.CreateDirectory(directory);
            using (var form = new Form())
            using (var start = new System.Windows.Forms.Timer())
            using (var timeout = new System.Windows.Forms.Timer())
            {
                form.Text = "Memory session replay smoke test";
                form.ClientSize = new Size(800, 600);
                form.KeyPreview = true;
                int downs = 0, ups = 0, keys = 0;
                form.MouseDown += (s, e) => { if (e.Button == MouseButtons.Left && Math.Abs(e.X - 120) <= 1 && Math.Abs(e.Y - 160) <= 1) downs++; };
                form.MouseUp += (s, e) => { if (e.Button == MouseButtons.Left) ups++; };
                form.KeyDown += (s, e) => { if (e.KeyCode == Keys.A) keys++; };
                form.Show();
                form.Activate();
                var recording = new Recording { Width = 800, Height = 600, Dpi = Native.GetDpiForWindow(form.Handle), Complete = true, DurationMs = 3500 };
                recording.Events.Add(new InputEvent { Ms = 500, Kind = "move", X = 120, Y = 160 });
                recording.Events.Add(new InputEvent { Ms = 600, Kind = "down", Code = 1, X = 120, Y = 160 });
                recording.Events.Add(new InputEvent { Ms = 700, Kind = "up", Code = 1, X = 120, Y = 160 });
                recording.Events.Add(new InputEvent { Ms = 1000, Kind = "keyDown", Code = 65 });
                recording.Events.Add(new InputEvent { Ms = 1100, Kind = "keyUp", Code = 65 });
                recording.Events.Add(new InputEvent { Ms = 1800, Kind = "checkpoint" });
                string path = Path.Combine(directory, "smoke-input.json");
                Session.Save(path, recording);
                using (var session = new Session(Process.GetCurrentProcess().Id, "replay", path, directory, true))
                {
                    start.Interval = 500;
                    start.Tick += (s, e) => { start.Stop(); session.StartOrResume(); };
                    start.Start();
                    timeout.Interval = 15000;
                    timeout.Tick += (s, e) => { timeout.Stop(); Application.ExitThread(); };
                    timeout.Start();
                    session.Execute();
                }
                if (downs != 1 || ups != 1 || keys != 1) throw new Exception("Replay delivery mismatch: " + downs + "/" + ups + "/" + keys);
                if (!File.ReadAllText(Path.Combine(directory, "result.txt")).Contains("outcome=replay_finished")) throw new Exception("Replay did not complete.");
                if (File.ReadAllLines(Path.Combine(directory, "memory.csv")).Length < 3) throw new Exception("Memory samples missing.");
                Console.WriteLine("Replay smoke passed: mouse coordinates, button pair, keyboard, checkpoint, screenshots and memory CSV.");
            }
        }
    }
}
