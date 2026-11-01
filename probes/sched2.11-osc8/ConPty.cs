using System;
using System.ComponentModel;
using System.IO;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading;
using Microsoft.Win32.SafeHandles;

// A child on a ConPTY with its output bytes kept: the probe's stand-in for Supraman's terminal.
public sealed class ConPtyProbe : IDisposable
{
    [StructLayout(LayoutKind.Sequential)] struct COORD { public short X, Y; }
    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
    struct STARTUPINFO
    {
        public int cb; public string lpReserved, lpDesktop, lpTitle;
        public int dwX, dwY, dwXSize, dwYSize, dwXCountChars, dwYCountChars, dwFillAttribute, dwFlags;
        public short wShowWindow, cbReserved2; public IntPtr lpReserved2, hStdInput, hStdOutput, hStdError;
    }
    [StructLayout(LayoutKind.Sequential)] struct STARTUPINFOEX { public STARTUPINFO StartupInfo; public IntPtr lpAttributeList; }
    [StructLayout(LayoutKind.Sequential)] struct PROCESS_INFORMATION { public IntPtr hProcess, hThread; public int dwProcessId, dwThreadId; }

    [DllImport("kernel32.dll", SetLastError = true)] static extern bool CreatePipe(out SafeFileHandle r, out SafeFileHandle w, IntPtr sa, int size);
    [DllImport("kernel32.dll", SetLastError = true)] static extern int CreatePseudoConsole(COORD size, SafeFileHandle input, SafeFileHandle output, uint flags, out IntPtr hpc);
    [DllImport("kernel32.dll", SetLastError = true)] static extern void ClosePseudoConsole(IntPtr hpc);
    [DllImport("kernel32.dll", SetLastError = true)] static extern bool InitializeProcThreadAttributeList(IntPtr list, int count, int flags, ref IntPtr size);
    [DllImport("kernel32.dll", SetLastError = true)] static extern bool UpdateProcThreadAttribute(IntPtr list, uint flags, IntPtr attr, IntPtr value, IntPtr size, IntPtr prev, IntPtr retSize);
    [DllImport("kernel32.dll", SetLastError = true)] static extern void DeleteProcThreadAttributeList(IntPtr list);
    [DllImport("kernel32.dll", SetLastError = true, CharSet = CharSet.Unicode)]
    static extern bool CreateProcessW(string app, StringBuilder cmd, IntPtr pa, IntPtr ta, bool inherit, uint flags, IntPtr env, string cwd, ref STARTUPINFOEX si, out PROCESS_INFORMATION pi);
    [DllImport("kernel32.dll")] static extern bool CloseHandle(IntPtr h);

    const uint EXTENDED_STARTUPINFO_PRESENT = 0x00080000;
    static readonly IntPtr PSEUDOCONSOLE = (IntPtr)0x00020016;

    IntPtr _hpc, _attrs;
    FileStream _in, _out;
    readonly MemoryStream _bytes = new MemoryStream();
    Thread _reader;
    public int Pid;

    public ConPtyProbe(string commandLine, string cwd, short cols, short rows)
    {
        SafeFileHandle inRead, inWrite, outRead, outWrite;
        if (!CreatePipe(out inRead, out inWrite, IntPtr.Zero, 0) || !CreatePipe(out outRead, out outWrite, IntPtr.Zero, 0))
            throw new Win32Exception();
        int hr = CreatePseudoConsole(new COORD { X = cols, Y = rows }, inRead, outWrite, 0, out _hpc);
        if (hr != 0) throw new Win32Exception(hr);
        inRead.Dispose(); outWrite.Dispose();
        _in = new FileStream(inWrite, FileAccess.Write);
        _out = new FileStream(outRead, FileAccess.Read);

        IntPtr size = IntPtr.Zero;
        InitializeProcThreadAttributeList(IntPtr.Zero, 1, 0, ref size);
        _attrs = Marshal.AllocHGlobal(size);
        if (!InitializeProcThreadAttributeList(_attrs, 1, 0, ref size)) throw new Win32Exception();
        if (!UpdateProcThreadAttribute(_attrs, 0, PSEUDOCONSOLE, _hpc, (IntPtr)IntPtr.Size, IntPtr.Zero, IntPtr.Zero)) throw new Win32Exception();

        var si = new STARTUPINFOEX();
        si.StartupInfo.cb = Marshal.SizeOf<STARTUPINFOEX>();
        si.lpAttributeList = _attrs;
        // Null std handles: without this the child inherits the caller's redirected pipes, not the ConPTY.
        si.StartupInfo.dwFlags = 0x00000100;
        PROCESS_INFORMATION pi;
        if (!CreateProcessW(null, new StringBuilder(commandLine), IntPtr.Zero, IntPtr.Zero, false, EXTENDED_STARTUPINFO_PRESENT, IntPtr.Zero, cwd, ref si, out pi))
            throw new Win32Exception();
        CloseHandle(pi.hThread); CloseHandle(pi.hProcess);
        Pid = pi.dwProcessId;

        _reader = new Thread(() =>
        {
            var buf = new byte[8192];
            try { int n; while ((n = _out.Read(buf, 0, buf.Length)) > 0) lock (_bytes) _bytes.Write(buf, 0, n); }
            catch (Exception) { }
        }) { IsBackground = true };
        _reader.Start();
    }

    public void Write(string text)
    {
        var b = Encoding.UTF8.GetBytes(text);
        _in.Write(b, 0, b.Length); _in.Flush();
    }

    public byte[] Bytes() { lock (_bytes) return _bytes.ToArray(); }

    public string Text() { return Encoding.UTF8.GetString(Bytes()); }

    public void Dispose()
    {
        try { _in.Dispose(); } catch (Exception) { }
        if (_hpc != IntPtr.Zero) { ClosePseudoConsole(_hpc); _hpc = IntPtr.Zero; }
        if (_attrs != IntPtr.Zero) { DeleteProcThreadAttributeList(_attrs); Marshal.FreeHGlobal(_attrs); _attrs = IntPtr.Zero; }
        _reader.Join(2000);
    }
}
