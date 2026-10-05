using System.Text.Json.Nodes;
using Microsoft.CodeAnalysis;
using Microsoft.CodeAnalysis.Text;

namespace Codeclew.CSharp.Analyzer;

/// <summary>
/// Source coordinates for one document: UTF-16 offsets into the decoded text,
/// one-based lines, and UTF-8 byte offsets into the original file bytes
/// (including a leading byte-order mark, which the decoded text omits).
/// </summary>
public sealed class SourceAnchors
{
    private readonly SourceText _text;
    private readonly int[] _utf8;

    public string File { get; }

    public SourceAnchors(string file, SourceText text, int bomBytes)
    {
        File = file;
        _text = text;
        _utf8 = new int[text.Length + 1];
        var bytes = bomBytes;
        for (var i = 0; i < text.Length; i++)
        {
            _utf8[i] = bytes;
            var c = text[i];
            if (char.IsHighSurrogate(c) && i + 1 < text.Length && char.IsLowSurrogate(text[i + 1]))
            {
                bytes += 4;
                _utf8[++i] = bytes;
                continue;
            }
            bytes += c <= 0x7f ? 1 : c <= 0x7ff ? 2 : 3;
        }
        _utf8[text.Length] = bytes;
    }

    public static int BomBytes(string path)
    {
        Span<byte> prefix = stackalloc byte[3];
        using var stream = System.IO.File.OpenRead(path);
        var read = stream.Read(prefix);
        return read == 3 && prefix[0] == 0xEF && prefix[1] == 0xBB && prefix[2] == 0xBF ? 3 : 0;
    }

    public void Apply(JsonObject row, TextSpan span)
    {
        row["file"] = File;
        row["start"] = span.Start;
        row["end"] = span.End;
        row["startLine"] = _text.Lines.GetLineFromPosition(span.Start).LineNumber + 1;
        row["endLine"] = _text.Lines.GetLineFromPosition(Math.Max(span.Start, span.End - 1)).LineNumber + 1;
        row["byteStart"] = _utf8[span.Start];
        row["byteEnd"] = _utf8[span.End];
    }
}
