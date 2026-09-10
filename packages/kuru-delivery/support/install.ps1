#requires -Version 5.1
[CmdletBinding()]
param(
    [string]$Version,
    [string]$Target,
    [string]$ReleaseBase,
    [string]$InstallDir,
    [switch]$Recover
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version 2.0

# This is the compiler-free entrypoint. Add-Type uses the compiler shipped with
# stock Windows PowerShell/.NET, never a downloaded compiler or candidate exe.
# The small native bridge retains authority across path checks and publication.
# CreateFile/MoveFileEx/LockFileEx contracts:
# https://learn.microsoft.com/windows/win32/api/fileapi/nf-fileapi-createfilew
# https://learn.microsoft.com/windows/win32/api/fileapi/nf-fileapi-lockfileex
# https://learn.microsoft.com/windows/win32/api/winbase/nf-winbase-movefileexw
if (-not ('Kuru.Bootstrap.Native' -as [type])) {
Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.ComponentModel;
using System.Diagnostics;
using System.IO;
using System.IO.Compression;
using System.Net;
using System.Runtime.InteropServices;
using System.Security.AccessControl;
using System.Security.Cryptography;
using System.Security.Principal;
using System.Text;
using Microsoft.Win32.SafeHandles;

namespace Kuru.Bootstrap {
public static class Native {
    public const int Limit = 128 * 1024 * 1024;
    const uint Read = 0x80000000, Write = 0x40000000, ReadControl = 0x20000;
    const uint NoFollow = 0x00200000, DirectoryFlag = 0x02000000;
    const uint ShareRead = 1, ShareWrite = 2, ShareDelete = 4;
    [StructLayout(LayoutKind.Sequential)] struct SecurityAttributes {
        public int size; public IntPtr descriptor; public int inherit;
    }
    [StructLayout(LayoutKind.Sequential)] struct FileTime { public uint low, high; }
    [StructLayout(LayoutKind.Sequential)] struct FileInfo {
        public uint attributes; public FileTime created, accessed, written;
        public uint volume, sizeHigh, sizeLow, links, indexHigh, indexLow;
    }
    [StructLayout(LayoutKind.Sequential)] struct Overlapped {
        public UIntPtr internalLow, internalHigh; public uint offset, offsetHigh; public IntPtr signal;
    }
    [DllImport("kernel32.dll", CharSet=CharSet.Unicode, SetLastError=true)]
    static extern SafeFileHandle CreateFileW(string path, uint access, uint share, IntPtr security, uint disposition, uint flags, IntPtr template);
    [DllImport("kernel32.dll", CharSet=CharSet.Unicode, SetLastError=true)]
    static extern bool CreateDirectoryW(string path, ref SecurityAttributes security);
    [DllImport("kernel32.dll", CharSet=CharSet.Unicode, SetLastError=true)]
    static extern bool MoveFileExW(string source, string target, uint flags);
    [DllImport("kernel32.dll", CharSet=CharSet.Unicode, SetLastError=true)]
    static extern bool DeleteFileW(string path);
    [DllImport("kernel32.dll", CharSet=CharSet.Unicode, SetLastError=true)]
    static extern bool RemoveDirectoryW(string path);
    [DllImport("kernel32.dll", SetLastError=true)]
    static extern bool GetFileInformationByHandle(SafeFileHandle handle, out FileInfo info);
    [DllImport("kernel32.dll", SetLastError=true)]
    static extern bool GetFileInformationByHandleEx(SafeFileHandle handle, int kind, byte[] data, uint size);
    [DllImport("kernel32.dll", SetLastError=true)] static extern uint GetFileType(SafeFileHandle handle);
    [DllImport("kernel32.dll", SetLastError=true)]
    static extern bool LockFileEx(SafeFileHandle handle, uint flags, uint reserved, uint low, uint high, ref Overlapped overlapped);
    [DllImport("advapi32.dll", CharSet=CharSet.Unicode, SetLastError=true)]
    static extern bool ConvertStringSecurityDescriptorToSecurityDescriptorW(string text, uint revision, out IntPtr descriptor, out uint size);
    [DllImport("advapi32.dll")] static extern uint GetSecurityInfo(SafeFileHandle handle, int kind, uint fields, out IntPtr owner, out IntPtr group, out IntPtr dacl, out IntPtr sacl, out IntPtr descriptor);
    [DllImport("advapi32.dll")] static extern uint GetSecurityDescriptorLength(IntPtr descriptor);
    [DllImport("kernel32.dll")] static extern IntPtr LocalFree(IntPtr value);

    static Exception Error(string message) { return new IOException(message, new Win32Exception(Marshal.GetLastWin32Error())); }
    static void Require(bool condition, string message) { if (!condition) throw new InvalidDataException(message); }
    static string Extended(string path) { return "\\\\?\\" + path; }
    public static string PathName(string path) {
        Require(!String.IsNullOrWhiteSpace(path) && path.IndexOfAny(new char[]{'\0','\r','\n'}) < 0, "invalid installation path");
        if (path.StartsWith("\\\\?\\", StringComparison.Ordinal)) path = path.Substring(4);
        path = path.Replace('/','\\');
        // Framework GetFullPath trims trailing periods/spaces. Reject native
        // aliases before normalization can erase that evidence; explicit dot
        // navigation is resolved only after the remaining components pass.
        string[] raw = path.Split('\\');
        for (int i=0;i<raw.Length;i++) {
            if (i==0 && raw[i].Length==2 && Char.IsLetter(raw[i][0]) && raw[i][1]==':' && path.Length>2 && path[2]=='\\') continue;
            if (raw[i].Length==0 || raw[i]=="." || raw[i]=="..") continue;
            Component(raw[i]);
        }
        string full = Path.GetFullPath(path).TrimEnd('\\');
        Require(full.Length >= 3 && full[1] == ':' && full[2] == '\\' && Char.IsLetter(full[0]) && full.Length < 32000, "installation requires a local absolute drive path");
        foreach (string part in full.Substring(3).Split('\\')) Component(part);
        return full;
    }
    public static void Component(string value) {
        Require(!String.IsNullOrEmpty(value) && value != "." && value != ".." && value.Length <= 255 && value.IndexOfAny(new char[]{'\\','/',':','\0','\r','\n','"','<','>','|','?','*'}) < 0 && !value.EndsWith(".") && !value.EndsWith(" "), "ambiguous Windows path component");
        foreach (char c in value) Require(c >= 32, "control character in path");
        string stem = value.Split('.')[0].ToUpperInvariant();
        Require(stem != "CON" && stem != "PRN" && stem != "AUX" && stem != "NUL" && stem != "CONIN$" && stem != "CONOUT$" && !(stem.Length == 4 && (stem.StartsWith("COM") || stem.StartsWith("LPT")) && "123456789\u00b9\u00b2\u00b3".IndexOf(stem[3]) >= 0), "reserved Windows device path");
    }
    static SafeFileHandle Open(string path, bool directory, bool write, uint disposition, bool movable) {
        SafeFileHandle handle = CreateFileW(Extended(path), ReadControl | (directory ? 0u : Read) | (write ? Write : 0u), ShareRead | (directory || write ? ShareWrite : 0u) | (movable ? ShareDelete : 0u), IntPtr.Zero, disposition, NoFollow | (directory ? DirectoryFlag : 0u), IntPtr.Zero);
        if (handle.IsInvalid) { handle.Dispose(); throw Error("cannot open checked path: " + path); }
        try { Info(handle, directory); return handle; } catch { handle.Dispose(); throw; }
    }
    static FileInfo Info(SafeFileHandle handle, bool directory) {
        FileInfo info;
        if (!GetFileInformationByHandle(handle, out info)) throw Error("cannot inspect file metadata");
        Require(GetFileType(handle) == 1 && (info.attributes & 0x400) == 0 && ((info.attributes & 0x10) != 0) == directory, "reparse points or non-regular objects are forbidden");
        if (!directory) Require(info.links == 1, "hard-linked files are forbidden");
        return info;
    }
    public static byte[] Identity(SafeFileHandle handle) {
        byte[] value = new byte[24];
        if (!GetFileInformationByHandleEx(handle, 18, value, 24)) throw Error("filesystem cannot supply full native identity");
        return value;
    }
    public static bool Equal(byte[] left, byte[] right) {
        if (left == null || right == null || left.Length != right.Length) return false;
        int difference = 0; for (int i = 0; i < left.Length; i++) difference |= left[i] ^ right[i];
        return difference == 0;
    }
    static void Private(SafeFileHandle handle, bool protectedDacl) {
        IntPtr owner, group, dacl, sacl, descriptor;
        uint error = GetSecurityInfo(handle, 1, 5, out owner, out group, out dacl, out sacl, out descriptor);
        if (error != 0) throw new IOException("cannot inspect private ACL", new Win32Exception((int)error));
        try {
            uint length = GetSecurityDescriptorLength(descriptor);
            Require(length > 0 && length <= 65536, "invalid security descriptor length");
            byte[] bytes = new byte[length]; Marshal.Copy(descriptor, bytes, 0, bytes.Length);
            RawSecurityDescriptor sd = new RawSecurityDescriptor(bytes, 0);
            Require(sd.DiscretionaryAcl != null && (!protectedDacl || (sd.ControlFlags & ControlFlags.DiscretionaryAclProtected) != 0), "private ACL must be present and protected");
            using (WindowsIdentity token = WindowsIdentity.GetCurrent()) {
                bool ownerRights = false, userAccess = false;
                foreach (GenericAce ace in sd.DiscretionaryAcl) {
                    CommonAce common = ace as CommonAce;
                    Require(common != null && !common.IsCallback, "unsupported private ACL entry");
                    if ((common.AceFlags & AceFlags.InheritOnly) != 0) continue;
                    Require(common.AceQualifier == AceQualifier.AccessAllowed || common.AceQualifier == AceQualifier.AccessDenied, "unsupported private ACL qualifier");
                    if (common.AceQualifier == AceQualifier.AccessDenied) continue;
                    if (common.SecurityIdentifier.Value == "S-1-3-4") { Require(common.AccessMask == 0, "owner-rights entry grants access"); ownerRights = true; }
                    else if (common.AccessMask != 0) { Require(common.SecurityIdentifier.Equals(token.User), "private ACL grants another principal access"); userAccess = true; }
                }
                Require(userAccess && sd.Owner != null && (sd.Owner.Equals(token.User) || (sd.Owner.Equals(token.Owner) && ownerRights)), "private ACL owner is not the current user or restricted token owner");
            }
        } finally { if (descriptor != IntPtr.Zero) LocalFree(descriptor); }
    }
    static void NewDirectory(string path, bool privateDirectory) {
        IntPtr descriptor = IntPtr.Zero;
        try {
            if (privateDirectory) using (WindowsIdentity token = WindowsIdentity.GetCurrent()) {
                uint length;
                if (!ConvertStringSecurityDescriptorToSecurityDescriptorW("O:" + token.User.Value + "D:P(A;OICI;FA;;;" + token.User.Value + ")(A;OICI;0x00000000;;;OW)", 1, out descriptor, out length)) throw Error("cannot construct private directory ACL");
            }
            SecurityAttributes security = new SecurityAttributes(); security.size = Marshal.SizeOf(typeof(SecurityAttributes)); security.descriptor = descriptor;
            if (!CreateDirectoryW(Extended(path), ref security)) {
                int code = Marshal.GetLastWin32Error();
                if (code != 183) throw new IOException("cannot create directory", new Win32Exception(code));
            }
        } finally { if (descriptor != IntPtr.Zero) LocalFree(descriptor); }
    }
    public sealed class DirectoryLease : IDisposable {
        readonly List<SafeFileHandle> handles = new List<SafeFileHandle>();
        public readonly string Path;
        public bool PublicationUncertain { get; internal set; }
        public byte[] Id { get { return Identity(handles[handles.Count - 1]); } }
        public DirectoryLease(string path, bool create, bool privateFinal) {
            Path = PathName(path);
            try {
                string current = Path.Substring(0, 3);
                handles.Add(Open(current, true, false, 3, false));
                string[] parts = Path.Substring(3).Split('\\');
                for (int i = 0; i < parts.Length; i++) {
                    current = System.IO.Path.Combine(current, parts[i]);
                    if (create) NewDirectory(current, privateFinal && i == parts.Length - 1);
                    SafeFileHandle handle = Open(current, true, false, 3, false);
                    handles.Add(handle);
                    if (privateFinal && i == parts.Length - 1) Private(handle, true);
                }
            } catch { Dispose(); throw; }
        }
        public string Child(string name) { Component(name); return System.IO.Path.Combine(Path, name); }
        public void Verify() {
            using (SafeFileHandle check = Open(Path, true, false, 3, false)) Require(Equal(Id, Identity(check)), "held directory identity changed");
        }
        public void Dispose() { for (int i = handles.Count - 1; i >= 0; i--) handles[i].Dispose(); handles.Clear(); }
    }
    public sealed class FileLease : IDisposable {
        public readonly FileStream Stream;
        public readonly byte[] Id;
        public FileLease(string path, bool privateFile, bool write, uint disposition) : this(path,privateFile,write,disposition,true) {}
        internal FileLease(string path, bool privateFile, bool write, uint disposition, bool movable) {
            SafeFileHandle handle = Open(path, false, write, disposition, movable);
            try {
                if (privateFile) Private(handle, false);
                Id = Identity(handle);
                Stream = new FileStream(handle, write ? FileAccess.ReadWrite : FileAccess.Read, 65536, false);
            } catch { handle.Dispose(); throw; }
        }
        public void Dispose() { Stream.Dispose(); }
    }
    public static bool Exists(string path) {
        // GetFileAttributes does not hide dangling reparse points like File.Exists.
        using (SafeFileHandle handle = CreateFileW(Extended(path), ReadControl, ShareRead | ShareWrite | ShareDelete, IntPtr.Zero, 3, NoFollow | DirectoryFlag, IntPtr.Zero)) {
            if (!handle.IsInvalid) return true;
            int code = Marshal.GetLastWin32Error();
            if (code == 2 || code == 3) return false;
            throw new IOException("cannot inspect destination", new Win32Exception(code));
        }
    }
    public static FileLease Lock(DirectoryLease state) {
        state.Verify();
        FileLease file = new FileLease(state.Child("install.lock"), true, true, 4, false);
        try {
            Overlapped overlapped = new Overlapped();
            // Rust File::try_lock locks the same entire file range on Windows.
            if (!LockFileEx(file.Stream.SafeFileHandle, 3, 0, UInt32.MaxValue, UInt32.MaxValue, ref overlapped)) throw Error("another update or recovery owns the installation");
            using (FileLease check = new FileLease(state.Child("install.lock"), true, true, 3, false)) Require(Equal(file.Id, check.Id), "installation lock identity changed");
            return file;
        } catch { file.Dispose(); throw; }
    }
    public static byte[] ReadBytes(FileLease file, int limit) {
        Require(file.Stream.Length <= limit, "file exceeds size limit"); file.Stream.Position = 0;
        using (MemoryStream output = new MemoryStream()) { Copy(file.Stream, output, limit); return output.ToArray(); }
    }
    static long Copy(Stream input, Stream output, int limit) { return Copy(input,output,limit,Stopwatch.StartNew()); }
    static long Copy(Stream input, Stream output, int limit, Stopwatch clock) {
        byte[] buffer = new byte[65536]; long count = 0;
        for (;;) {
            Require(clock.Elapsed < TimeSpan.FromSeconds(60), "stream exceeded bounded completion deadline");
            int read = input.Read(buffer, 0, Math.Min(buffer.Length, (int)(limit - count + 1)));
            if (read == 0) break;
            count += read; Require(count <= limit, "stream exceeds size limit"); output.Write(buffer, 0, read);
        }
        return count;
    }
    public static string Hash(byte[] value) { using (SHA256 hash = SHA256.Create()) return BitConverter.ToString(hash.ComputeHash(value)).Replace("-", "").ToLowerInvariant(); }
    public static string FileHash(FileLease file) { file.Stream.Position = 0; using (SHA256 hash = SHA256.Create()) return BitConverter.ToString(hash.ComputeHash(file.Stream)).Replace("-", "").ToLowerInvariant(); }
    public static byte[] Fetch(string origin, string name, int limit) {
        Component(name);
        if (!origin.StartsWith("https://", StringComparison.OrdinalIgnoreCase)) {
            using (DirectoryLease directory = new DirectoryLease(origin, false, false))
            using (FileLease file = new FileLease(directory.Child(name), false, false, 3)) return ReadBytes(file, limit);
        }
        Uri uri = new Uri(origin.TrimEnd('/') + "/" + name); Stopwatch deadline = Stopwatch.StartNew();
        for (int redirect = 0; redirect <= 8; redirect++) {
            Require(deadline.Elapsed < TimeSpan.FromSeconds(60), "download exceeded completion deadline");
            Require(uri.Scheme == "https" && String.IsNullOrEmpty(uri.UserInfo) && String.IsNullOrEmpty(uri.Fragment), "download redirect is not plain HTTPS");
            HttpWebRequest request = (HttpWebRequest)WebRequest.Create(uri);
            request.AllowAutoRedirect = false; request.Timeout = 15000; request.ReadWriteTimeout = 15000;
            request.UseDefaultCredentials = false; request.Credentials = null; request.AutomaticDecompression = DecompressionMethods.None;
            request.UserAgent = "kuru-bootstrap";
            try {
                using (HttpWebResponse response = (HttpWebResponse)request.GetResponse()) {
                    int status = (int)response.StatusCode;
                    if (status >= 300 && status <= 399) { Require(redirect < 8 && response.Headers["Location"] != null, "too many or invalid download redirects"); uri = new Uri(uri, response.Headers["Location"]); continue; }
                    Require(status == 200 && response.ContentLength <= limit && deadline.Elapsed < TimeSpan.FromSeconds(60), "download failed or exceeds bounds");
                    using (Stream input = response.GetResponseStream()) using (MemoryStream output = new MemoryStream()) { Copy(input, output, limit,deadline); return output.ToArray(); }
                }
            } finally { request.Abort(); }
        }
        throw new IOException("download redirect limit exceeded");
    }
    static ushort U16(byte[] data, int at) { Require(at >= 0 && at <= data.Length - 2, "truncated ZIP record"); return BitConverter.ToUInt16(data, at); }
    static uint U32(byte[] data, int at) { Require(at >= 0 && at <= data.Length - 4, "truncated binary record"); return BitConverter.ToUInt32(data, at); }
    sealed class Record { public string name; public int offset, start, compressed, expanded; public uint crc; public ushort method; }
    // RFC 1951 framing only: stock .NET still performs payload decompression.
    // Framework DeflateStream can stop at source EOF without exposing whether
    // BFINAL completed, and may read ahead past it. Independently account for
    // every bit and expanded length before using that decoder. No reflection,
    // private runtime API or native zlib installation is required.
    sealed class Bits {
        readonly byte[] bytes; readonly int start, length; int position;
        public Bits(byte[] bytes, int start, int length) { this.bytes=bytes; this.start=start; this.length=length; }
        public int Read(int count) {
            Require(count >= 0 && count <= 16 && position <= length * 8 - count, "truncated DEFLATE block");
            int value=0; for (int i=0;i<count;i++,position++) value |= ((bytes[start+(position>>3)] >> (position&7)) & 1) << i;
            return value;
        }
        public int Stored() {
            position = (position+7) & ~7;
            int count=Read(16), complement=Read(16);
            Require((count ^ complement) == 65535 && position <= length*8-count*8, "invalid stored DEFLATE block");
            position += count*8; return count;
        }
        public void End() { Require((position+7)/8 == length, "trailing bytes after final DEFLATE block"); }
    }
    sealed class Huffman {
        readonly Dictionary<int,int> symbols = new Dictionary<int,int>();
        public Huffman(int[] lengths) {
            int[] counts=new int[16], next=new int[16];
            foreach(int size in lengths) { Require(size>=0 && size<=15,"invalid DEFLATE code length"); if(size!=0) counts[size]++; }
            int left=1, code=0;
            for(int size=1;size<=15;size++) { left=left*2-counts[size]; Require(left>=0,"oversubscribed DEFLATE tree"); code=(code+counts[size-1])*2; next[size]=code; }
            for(int symbol=0;symbol<lengths.Length;symbol++) {
                int size=lengths[symbol]; if(size==0) continue;
                int forward=next[size]++, reverse=0;
                for(int i=0;i<size;i++) { reverse=(reverse<<1)|(forward&1); forward>>=1; }
                symbols.Add((1<<size)|reverse,symbol);
            }
        }
        public int Read(Bits bits) {
            int code=0, symbol;
            for(int size=1;size<=15;size++) { code |= bits.Read(1) << (size-1); if(symbols.TryGetValue((1<<size)|code,out symbol)) return symbol; }
            throw new InvalidDataException("invalid DEFLATE Huffman code");
        }
    }
    static readonly int[] LengthBase={3,4,5,6,7,8,9,10,11,13,15,17,19,23,27,31,35,43,51,59,67,83,99,115,131,163,195,227,258};
    static readonly int[] LengthExtra={0,0,0,0,0,0,0,0,1,1,1,1,2,2,2,2,3,3,3,3,4,4,4,4,5,5,5,5,0};
    static readonly int[] DistanceBase={1,2,3,4,5,7,9,13,17,25,33,49,65,97,129,193,257,385,513,769,1025,1537,2049,3073,4097,6145,8193,12289,16385,24577};
    static readonly int[] DistanceExtra={0,0,0,0,1,1,2,2,3,3,4,4,5,5,6,6,7,7,8,8,9,9,10,10,11,11,12,12,13,13};
    static void DeflateFrame(byte[] bytes, Record record) {
        Bits bits=new Bits(bytes,record.start,record.compressed); long expanded=0; bool final=false;
        Stopwatch deadline=Stopwatch.StartNew(); int work=0;
        while(!final) {
            Require(deadline.Elapsed<TimeSpan.FromSeconds(60),"DEFLATE framing exceeded deadline");
            final=bits.Read(1)!=0; int kind=bits.Read(2);
            if(kind==0) expanded+=bits.Stored();
            else {
                Require(kind==1 || kind==2,"reserved DEFLATE block type");
                int[] literals, distances;
                if(kind==1) {
                    literals=new int[288]; distances=new int[32];
                    for(int n=0;n<288;n++) literals[n]=n<144?8:n<256?9:n<280?7:8;
                    for(int n=0;n<32;n++) distances[n]=5;
                } else {
                    int literalCount=bits.Read(5)+257, distanceCount=bits.Read(5)+1, codeCount=bits.Read(4)+4;
                    Require(literalCount<=286,"reserved DEFLATE literal code count");
                    // The wire order is fixed by RFC1951, independent of trees.
                    int[] order={16,17,18,0,8,7,9,6,10,5,11,4,12,3,13,2,14,1,15};
                    int[] codes=new int[19]; for(int n=0;n<codeCount;n++) codes[order[n]]=bits.Read(3);
                    Huffman lengths=new Huffman(codes); int[] values=new int[literalCount+distanceCount]; int at=0;
                    while(at<values.Length) {
                        int symbol=lengths.Read(bits);
                        if(symbol<=15) values[at++]=symbol;
                        else {
                            Require(symbol<=18 && (symbol!=16 || at>0),"invalid repeated DEFLATE code length");
                            int count=symbol==16?bits.Read(2)+3:symbol==17?bits.Read(3)+3:bits.Read(7)+11;
                            int value=symbol==16?values[at-1]:0;
                            Require(count<=values.Length-at,"DEFLATE code repeat exceeds tree");
                            for(int n=0;n<count;n++) values[at++]=value;
                        }
                    }
                    literals=new int[literalCount]; distances=new int[distanceCount]; Array.Copy(values,0,literals,0,literalCount); Array.Copy(values,literalCount,distances,0,distanceCount);
                    Require(literals[256]!=0,"DEFLATE tree has no end-of-block code");
                }
                Huffman literal=new Huffman(literals), distance=new Huffman(distances);
                for(;;) {
                    if((work++ & 4095)==0) Require(deadline.Elapsed<TimeSpan.FromSeconds(60),"DEFLATE framing exceeded deadline");
                    int symbol=literal.Read(bits);
                    if(symbol==256) break;
                    if(symbol<256) expanded++;
                    else {
                        Require(symbol<=285,"reserved DEFLATE length symbol");
                        int length=LengthBase[symbol-257]+bits.Read(LengthExtra[symbol-257]);
                        int code=distance.Read(bits); Require(code<30,"reserved DEFLATE distance symbol");
                        int offset=DistanceBase[code]+bits.Read(DistanceExtra[code]); Require(offset<=expanded,"DEFLATE backreference precedes output"); expanded+=length;
                    }
                    Require(expanded<=record.expanded,"DEFLATE output exceeds declared size");
                }
            }
            Require(expanded<=record.expanded,"DEFLATE output exceeds declared size");
        }
        bits.End(); Require(expanded==record.expanded,"DEFLATE framing output size mismatch");
    }
    static readonly uint[] CrcTable = MakeCrcTable();
    static uint[] MakeCrcTable() { uint[] table = new uint[256]; for (uint n = 0; n < 256; n++) { uint c = n; for (int k = 0; k < 8; k++) c = (c & 1) != 0 ? 0xedb88320 ^ (c >> 1) : c >> 1; table[n] = c; } return table; }
    public static byte[] Executable(byte[] bytes) {
        Require(bytes.Length <= Limit && bytes.Length >= 22, "ZIP exceeds bounds or is truncated");
        int end = bytes.Length - 22;
        Require(U32(bytes,end) == 0x06054b50 && U16(bytes,end+4) == 0 && U16(bytes,end+6) == 0 && U16(bytes,end+8) == 3 && U16(bytes,end+10) == 3 && U16(bytes,end+20) == 0, "ZIP must have exactly three members, one disk and no trailer");
        uint centralValue = U32(bytes,end+16), sizeValue = U32(bytes,end+12);
        Require((ulong)centralValue + sizeValue == (ulong)end, "ZIP central bounds disagree");
        int central = checked((int)centralValue), cursor = central; long total = 0;
        HashSet<string> names = new HashSet<string>(StringComparer.Ordinal); List<Record> records = new List<Record>();
        for (int index = 0; index < 3; index++) {
            Require(U32(bytes,cursor) == 0x02014b50 && bytes[cursor+5] == 3 && (U16(bytes,cursor+6) == 10 || U16(bytes,cursor+6) == 20), "unsupported ZIP header or creator");
            Require(U16(bytes,cursor+8) == 0 && U16(bytes,cursor+30) == 0 && U16(bytes,cursor+32) == 0 && U16(bytes,cursor+34) == 0 && U16(bytes,cursor+36) == 0, "ZIP flags, extra fields or comments are forbidden");
            int length = U16(bytes,cursor+28); Require(cursor <= end - 46 - length, "truncated ZIP name");
            string name = Encoding.ASCII.GetString(bytes,cursor+46,length);
            Require((name == "kuru.exe" || name == "LICENSE" || name == "README.md") && names.Add(name), "ZIP inventory must be exactly kuru.exe, LICENSE and README.md without duplicates");
            uint expectedMode = name == "kuru.exe" ? 0x81ed0000u : 0x81a40000u;
            Require(U32(bytes,cursor+38) == expectedMode, "ZIP entry is not a regular file with the expected mode");
            Record record = new Record(); record.name = name; record.offset = checked((int)U32(bytes,cursor+42)); record.method = U16(bytes,cursor+10); record.crc = U32(bytes,cursor+16); record.compressed = checked((int)U32(bytes,cursor+20)); record.expanded = checked((int)U32(bytes,cursor+24));
            Require((record.method == 0 || record.method == 8) && record.compressed <= Limit && record.expanded <= Limit && (record.method != 0 || record.compressed == record.expanded), "ZIP compression or member size is invalid");
            total += record.expanded; Require(total <= Limit, "expanded ZIP exceeds limit");
            Require(U32(bytes,record.offset) == 0x04034b50 && U16(bytes,record.offset+26) == length && U16(bytes,record.offset+28) == 0, "invalid ZIP local header");
            for (int n = 0; n < 22; n++) Require(bytes[record.offset+4+n] == bytes[cursor+6+n], "ZIP local metadata disagrees");
            for (int n = 0; n < length; n++) Require(bytes[record.offset+30+n] == bytes[cursor+46+n], "ZIP local names disagree");
            record.start = checked(record.offset + 30 + length); Require((long)record.start + record.compressed <= central, "ZIP data overlaps central directory"); records.Add(record); cursor += 46 + length;
        }
        Require(cursor == end, "unaccounted ZIP central bytes");
        records.Sort(delegate(Record a, Record b) { return a.offset.CompareTo(b.offset); });
        int next = 0; byte[] executable = null;
        foreach (Record record in records) {
            Require(record.offset == next, "unaccounted or overlapping ZIP physical records"); next = checked(record.start + record.compressed);
            if(record.method==8) DeflateFrame(bytes,record);
            using (MemoryStream input = new MemoryStream(bytes,record.start,record.compressed,false))
            using (Stream decoded = record.method == 8 ? (Stream)new DeflateStream(input,CompressionMode.Decompress,true) : input)
            using (MemoryStream output = new MemoryStream()) {
                long count = Copy(decoded,output,record.expanded);
                Require(count == record.expanded && input.Position == record.compressed, "ZIP decoded size or compressed stream boundary disagrees");
                byte[] value = output.ToArray(); uint crc = UInt32.MaxValue;
                foreach (byte item in value) crc = CrcTable[(crc ^ item) & 255] ^ (crc >> 8);
                Require((crc ^ UInt32.MaxValue) == record.crc, "ZIP member CRC mismatch");
                if (record.name == "kuru.exe") executable = value;
            }
        }
        Require(next == central && executable != null && executable.Length > 0, "ZIP executable is absent or physical bytes are unaccounted");
        Pe(executable); return executable;
    }
    public static void Pe(byte[] value) {
        Require(value.Length >= 64 && value[0] == 'M' && value[1] == 'Z', "release executable is not PE");
        int offset = checked((int)U32(value,60));
        Require(offset >= 64 && offset <= value.Length - 26 && U32(value,offset) == 0x00004550 && U16(value,offset+4) == 0x8664 && (U16(value,offset+22) & 2) != 0 && (U16(value,offset+22) & 0x2000) == 0 && U16(value,offset+24) == 0x20b, "release executable must be an x64 PE32+ executable, not a DLL");
        int optional = U16(value,offset+20), sections = U16(value,offset+6);
        Require(optional >= 112 && sections > 0 && sections <= 96 && (long)offset + 24 + optional + sections * 40 <= value.Length, "PE header bounds are invalid");
    }
    public static void Publish(DirectoryLease stage, DirectoryLease parent, byte[] bytes) { Publish(stage,parent,bytes,null); }
    // Explicit callback seam for the native post-move fault fixture. The public
    // bootstrap route never supplies a callback or reads an environment switch.
    public static void Publish(DirectoryLease stage, DirectoryLease parent, byte[] bytes, Action afterMove) {
        stage.Verify(); parent.Verify(); string source = stage.Child("kuru.exe"), target = parent.Child("kuru.exe");
        using (FileLease write = new FileLease(source,true,true,1)) { write.Stream.Write(bytes,0,bytes.Length); write.Stream.Flush(true); }
        using (FileLease candidate = new FileLease(source,true,false,3)) {
            Require(FileHash(candidate) == Hash(bytes), "staged executable changed"); byte[] previous = null;
            if (Exists(target)) using (FileLease old = new FileLease(target,false,false,3)) { previous = old.Id; }
            stage.Verify(); parent.Verify();
            stage.PublicationUncertain = true;
            bool moved = MoveFileExW(Extended(source),Extended(target),9); int error = Marshal.GetLastWin32Error();
            if (moved && afterMove != null) afterMove();
            if (Exists(target)) using (FileLease installed = new FileLease(target,false,false,3)) {
                if (Equal(installed.Id,candidate.Id) && FileHash(installed) == Hash(bytes)) {
                    if (!moved) throw new IOException("new bytes are installed but durable completion is uncertain",new Win32Exception(error));
                    stage.PublicationUncertain = false;
                    return;
                }
                if (!moved && Equal(installed.Id,previous)) { stage.PublicationUncertain=false; throw new IOException("publication failed; existing executable unchanged",new Win32Exception(error)); }
            }
            if (!moved && previous == null && !Exists(target) && Exists(source)) {
                using (FileLease unchanged = new FileLease(source,true,false,3)) {
                    if(Equal(unchanged.Id,candidate.Id)) { stage.PublicationUncertain=false; throw new IOException("publication failed; installation remains absent",new Win32Exception(error)); }
                }
            }
            throw new IOException("publication could not be reconciled; inspect the retained installation and stage",new Win32Exception(error));
        }
    }
    public static void RemoveStage(DirectoryLease stage) {
        stage.Verify(); string path = stage.Path;
        Require(!stage.PublicationUncertain, "publication is uncertain; checked stage retained at " + path);
        // Only the one known candidate can be deleted. Unknown entries retain
        // the stage and fail closed instead of recursive wildcard deletion.
        string candidate = stage.Child("kuru.exe");
        if (Exists(candidate)) using (FileLease file = new FileLease(candidate,true,false,3)) {
            byte[] identity = file.Id;
            using (FileLease check = new FileLease(candidate,true,false,3)) Require(Equal(identity,check.Id), "stage candidate identity changed");
            if (!DeleteFileW(Extended(candidate))) throw Error("cannot remove checked stage candidate");
        }
        stage.Dispose(); if (!RemoveDirectoryW(Extended(path))) throw Error("private stage retained after cleanup failure");
    }
    public static void RetireReceipt(DirectoryLease directory) {
        directory.Verify(); string path = directory.Child("receipt.json");
        using (FileLease held = new FileLease(path,true,false,3)) {
            using (FileLease check = new FileLease(path,true,false,3)) Require(Equal(held.Id,check.Id), "receipt identity changed before retirement");
            if (!DeleteFileW(Extended(path))) throw Error("cannot retire completed update receipt");
        }
        Require(!Exists(path), "completed receipt deletion remains uncertain"); directory.Verify();
    }
    public static string Quote(string argument) {
        StringBuilder output = new StringBuilder("\""); int slashes = 0;
        foreach (char value in argument) { if (value == '\\') { slashes++; continue; } if (value == '"') { output.Append('\\',slashes*2+1); output.Append(value); } else { output.Append('\\',slashes); output.Append(value); } slashes=0; }
        output.Append('\\',slashes*2); output.Append('"'); return output.ToString();
    }
    public static void Recover(string helper, string request, string directory) {
        ProcessStartInfo info = new ProcessStartInfo(helper,"--internal-update-helper " + Quote(request));
        info.UseShellExecute = false; info.WorkingDirectory = directory; info.EnvironmentVariables.Clear();
        string system = Environment.GetFolderPath(Environment.SpecialFolder.System);
        info.EnvironmentVariables["SystemRoot"] = System.IO.Directory.GetParent(system).FullName;
        info.EnvironmentVariables["PATH"] = system;
        string profile = Environment.GetEnvironmentVariable("LLVM_PROFILE_FILE"); if (profile != null) info.EnvironmentVariables["LLVM_PROFILE_FILE"] = profile;
        // This is verified current trusted recovery code with its own bounded
        // lease/cleanup lifecycle. It must survive creator loss; no candidate or
        // arbitrary command reaches this one deliberately unowned launch.
        using (Process child = Process.Start(info)) {
            if (!child.WaitForExit(30000)) throw new IOException("trusted recovery is still active; receipt retained (no process was killed)");
            Require(child.ExitCode == 0, "trusted update recovery failed; receipt retained");
        }
    }
}
}
'@
}

function Read-KuruReceipt($State, $Parent) {
    $receiptPath = $State.Child('receipt.json')
    if (-not [Kuru.Bootstrap.Native]::Exists($receiptPath)) { return $null }
    $file = [Kuru.Bootstrap.Native+FileLease]::new($receiptPath, $true, $false, 3)
    try {
        $bytes = [Kuru.Bootstrap.Native]::ReadBytes($file, 65536)
        $text = [Text.UTF8Encoding]::new($false, $true).GetString($bytes)
        $receipt = $text | ConvertFrom-Json
    } finally { $file.Dispose() }
    $allowed = @('schema_version','operation','parent','parent_identity','installed','displaced','candidate','backup','original','replacement','rollback','helper','helper_image','restored','phase')
    foreach ($property in $receipt.PSObject.Properties.Name) { if ($property -notin $allowed) { throw 'Unknown update receipt field.' } }
    $operation = [Guid]::Empty
    if ($receipt.schema_version -ne 1 -or -not [Guid]::TryParseExact($receipt.operation, 'D', [ref]$operation) -or
        $receipt.installed -cne 'kuru.exe' -or $receipt.displaced -cne ".kuru-old-$($receipt.operation).exe" -or
        $receipt.candidate -cne "candidate-$($receipt.operation).exe" -or $receipt.backup -cne "backup-$($receipt.operation).exe" -or
        -not [String]::Equals([Kuru.Bootstrap.Native]::PathName($receipt.parent), $Parent.Path, [StringComparison]::OrdinalIgnoreCase) -or
        -not [Kuru.Bootstrap.Native]::Equal([byte[]]$receipt.parent_identity, $Parent.Id) -or
        $receipt.phase -cnotin @('prepared','old_moved','published','rolled_back','cleanup_pending','complete')) { throw 'Update receipt identity or schema is invalid.' }
    foreach ($image in @($receipt.original, $receipt.replacement, $receipt.rollback, $receipt.helper_image)) {
        if ($image.identity.Count -ne 24 -or $image.sha256 -cnotmatch '^[0-9a-f]{64}$' -or $image.bytes -le 0 -or $image.bytes -gt [Kuru.Bootstrap.Native]::Limit) { throw 'Update image record is invalid.' }
        foreach ($property in $image.PSObject.Properties.Name) { if ($property -notin @('identity','sha256','bytes')) { throw 'Unknown update image field.' } }
    }
    return $receipt
}

$parent = $null; $state = $null; $lease = $null; $stage = $null
try {
    if ([Environment]::OSVersion.Platform -ne [PlatformID]::Win32NT -or -not [Environment]::Is64BitProcess -or $env:PROCESSOR_ARCHITECTURE -ne 'AMD64') { throw 'Use native 64-bit Windows PowerShell 5.1 on Windows x64.' }
    if (-not $Target) { $Target = 'x86_64-pc-windows-msvc' }
    if ($Target -cne 'x86_64-pc-windows-msvc') { throw 'This bootstrap installs the native Windows x64 target only.' }
    if (-not $InstallDir) { $InstallDir = $env:KURU_INSTALL_DIR }
    if (-not $InstallDir) {
        if (-not $env:LOCALAPPDATA) { throw 'Provide -InstallDir when LOCALAPPDATA is unset.' }
        $InstallDir = Join-Path $env:LOCALAPPDATA 'Programs/kuru/bin'
    }
    $parent = [Kuru.Bootstrap.Native+DirectoryLease]::new($InstallDir, $true, $false)
    $state = [Kuru.Bootstrap.Native+DirectoryLease]::new($parent.Child('.kuru-update'), $true, $true)
    $lease = [Kuru.Bootstrap.Native]::Lock($state)
    $receipt = Read-KuruReceipt $state $parent
    if ($null -ne $receipt -and $receipt.phase -cnotin @('complete','rolled_back')) {
        $helperPath = [Kuru.Bootstrap.Native]::PathName($receipt.helper)
        if ([IO.Path]::GetFileName($helperPath) -cne "x86_64-pc-windows-msvc-$($receipt.helper_image.sha256).exe" -or
            $receipt.helper_image.sha256 -cne $receipt.original.sha256 -or $receipt.helper_image.bytes -ne $receipt.original.bytes) { throw 'Receipt does not name a trusted original helper.' }
        $helperParent = [Kuru.Bootstrap.Native+DirectoryLease]::new([IO.Path]::GetDirectoryName($helperPath), $false, $true)
        $helper = $null
        try {
            $helper = [Kuru.Bootstrap.Native+FileLease]::new($helperPath, $true, $false, 3)
            if (-not [Kuru.Bootstrap.Native]::Equal($helper.Id, [byte[]]$receipt.helper_image.identity) -or $helper.Stream.Length -ne $receipt.helper_image.bytes -or [Kuru.Bootstrap.Native]::FileHash($helper) -cne $receipt.helper_image.sha256) { throw 'Trusted helper identity or checksum changed; refusing execution.' }
            $request = @{ recovery = $parent.Path } | ConvertTo-Json -Compress
            $lease.Dispose(); $lease = $null
            [Kuru.Bootstrap.Native]::Recover($helperPath, $request, $parent.Path)
        } finally { if ($null -ne $helper) { $helper.Dispose() }; $helperParent.Dispose() }
        $lease = [Kuru.Bootstrap.Native]::Lock($state)
        $receipt = Read-KuruReceipt $state $parent
        if ($null -eq $receipt -or $receipt.phase -cnotin @('complete','rolled_back')) { throw 'Update recovery remains unresolved; installation was not attempted.' }
    }
    if ($null -ne $receipt) {
            $expected = if ($receipt.phase -ceq 'complete') { $receipt.replacement } elseif ($receipt.PSObject.Properties.Name -contains 'restored' -and $null -ne $receipt.restored) { $receipt.restored } else { $receipt.original }
            $installed = [Kuru.Bootstrap.Native+FileLease]::new($parent.Child('kuru.exe'), $false, $false, 3)
            try {
                if (-not [Kuru.Bootstrap.Native]::Equal($installed.Id, [byte[]]$expected.identity) -or $installed.Stream.Length -ne $expected.bytes -or [Kuru.Bootstrap.Native]::FileHash($installed) -cne $expected.sha256) { throw 'Recovered executable does not match the recorded installed image.' }
            } finally { $installed.Dispose() }
    }
    if ($Recover) {
        Write-Output "Recovery complete for $($parent.Path)."
    } else {
        # A completed receipt identifies the old installed object. Retire it
        # only after verification, under the common lock, before a fresh install
        # establishes another identity. The immutable trusted helper cache stays.
        if ($null -ne $receipt) { [Kuru.Bootstrap.Native]::RetireReceipt($state) }
        if ($Version -and $Version.StartsWith('v')) { $Version = $Version.Substring(1) }
        if ($Version -and $Version -cnotmatch '^[0-9]+\.[0-9]+\.[0-9]+(-[a-zA-Z0-9.-]+)?$') { throw 'Version must be an explicit semantic version.' }
        if (-not $ReleaseBase) { $ReleaseBase = $env:KURU_RELEASE_BASE }
        $defaultBase = -not $ReleaseBase
        if ($defaultBase) {
            if ($Version) { $ReleaseBase = "https://github.com/replygirl/kuru/releases/download/v$Version" }
            else { $ReleaseBase = 'https://github.com/replygirl/kuru/releases/latest/download' }
        } elseif (-not $Version) { throw 'A custom release base requires -Version.' }
        if ($ReleaseBase -match '^[a-zA-Z][a-zA-Z0-9+.-]*://') {
            $uri = [Uri]$ReleaseBase
            if ($uri.Scheme -ne 'https' -or $uri.UserInfo -or $uri.Query -or $uri.Fragment -or $ReleaseBase -match '[\s\\]') { throw 'Release base must be HTTPS without credentials, whitespace, a query or fragment.' }
            [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
        } else { $ReleaseBase = [Kuru.Bootstrap.Native]::PathName($ReleaseBase) }
        $manifestBytes = [Kuru.Bootstrap.Native]::Fetch($ReleaseBase, 'SHA256SUMS', 65536)
        $manifest = [Text.UTF8Encoding]::new($false, $true).GetString($manifestBytes)
        if ($manifest.Contains([string][char]0)) { throw 'Checksum manifest contains NUL.' }
        $chosen = @()
        foreach ($line in ($manifest -split "`n")) {
            $line = $line.TrimEnd("`r")
            if (-not $line) { continue }
            if ($line -cnotmatch '^([0-9a-fA-F]{64}) [ *]([^\s]+)$') { throw 'Malformed checksum manifest.' }
            $hash = $Matches[1].ToLowerInvariant(); $name = $Matches[2]
            $pattern = '^kuru-([0-9]+\.[0-9]+\.[0-9]+(?:-[a-zA-Z0-9.-]+)?)-x86_64-pc-windows-msvc\.zip$'
            if ($name -cmatch $pattern) {
                $selected = $Matches[1]
                if (-not $Version -or $Version -ceq $selected) { $chosen += @{ version=$selected; name=$name; hash=$hash } }
            }
        }
        if ($chosen.Count -ne 1) { throw 'Checksum manifest must name the release archive exactly once.' }
        $Version = $chosen[0].version
        if ($defaultBase) { $ReleaseBase = "https://github.com/replygirl/kuru/releases/download/v$Version" }
        $archive = [Kuru.Bootstrap.Native]::Fetch($ReleaseBase, $chosen[0].name, [Kuru.Bootstrap.Native]::Limit)
        if ([Kuru.Bootstrap.Native]::Hash($archive) -cne $chosen[0].hash) { throw 'Release archive checksum mismatch; existing executable unchanged.' }
        Write-Output 'Verifying Kuru release archive.'
        $payload = [Kuru.Bootstrap.Native]::Executable($archive)
        $stage = [Kuru.Bootstrap.Native+DirectoryLease]::new($parent.Child(".kuru-install-$([Guid]::NewGuid().ToString('D'))"), $true, $true)
        [Kuru.Bootstrap.Native]::Publish($stage, $parent, $payload)
        Write-Output "Installed Kuru $Version at $($parent.Child('kuru.exe')); add $($parent.Path) to PATH."
    }
} finally {
    try {
        if ($null -ne $stage) {
            if ($stage.PublicationUncertain) { Write-Warning "Publication is uncertain; checked stage retained at $($stage.Path)." }
            else { [Kuru.Bootstrap.Native]::RemoveStage($stage) }
        }
    }
    finally {
        if ($null -ne $stage) { $stage.Dispose() }
        if ($null -ne $lease) { $lease.Dispose() }
        if ($null -ne $state) { $state.Dispose() }
        if ($null -ne $parent) { $parent.Dispose() }
    }
}
