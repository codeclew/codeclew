using System.Security.Cryptography;
using System.Text;
using System.Text.Json.Nodes;
using Microsoft.CodeAnalysis.Text;

namespace Codeclew.CSharp.Analyzer;

/// <summary>
/// NDJSON writer: one header line, one line per fact, and a trailer that binds the
/// fact count and the SHA-256 digest of every fact line (each followed by LF).
/// </summary>
public sealed class FactWriter : IDisposable
{
    public const string FactSchema = "codeclew-csharp-compiler-fact/1.0";
    public const string OutputSchema = "codeclew-csharp-analyzer-output/1.0";
    private const int MaxFactBytes = 256 * 1024;

    private readonly Stream _stream;
    private readonly IncrementalHash _digest = IncrementalHash.CreateHash(HashAlgorithmName.SHA256);
    private readonly int _maxFacts;
    private readonly SortedDictionary<string, int> _droppedBoundaries = new(StringComparer.Ordinal);

    public int FactCount { get; private set; }

    public FactWriter(string path, int maxFacts)
    {
        _stream = new FileStream(path, FileMode.CreateNew, FileAccess.Write, FileShare.None, 1 << 20);
        _maxFacts = maxFacts;
    }

    public JsonObject Row(string kind) => new() { ["schema"] = FactSchema, ["kind"] = kind };

    public void Header(JsonObject header)
    {
        header["schema"] = OutputSchema;
        WriteLine(Encoding.UTF8.GetBytes(header.ToJsonString()), digest: false);
    }

    public void Write(JsonObject fact)
    {
        StripNulls(fact);
        var bytes = Encoding.UTF8.GetBytes(fact.ToJsonString());
        if (bytes.Length > MaxFactBytes && fact["documentation"] is not null)
        {
            // Keep the declaration and drop its oversized flow, recording why.
            fact.Remove("documentation");
            CountBoundary("DOCUMENTATION_FLOW_FACT_BYTE_BUDGET");
            bytes = Encoding.UTF8.GetBytes(fact.ToJsonString());
        }
        if (bytes.Length > MaxFactBytes && fact["clrAttributes"] is not null)
        {
            fact.Remove("clrAttributes");
            CountBoundary("CLR_ATTRIBUTE_FACT_BYTE_BUDGET");
            bytes = Encoding.UTF8.GetBytes(fact.ToJsonString());
        }
        if (bytes.Length > MaxFactBytes)
        {
            throw new UnsupportedException("CSHARP_FACT_BYTE_LIMIT", "a C# fact exceeds 256 KiB");
        }
        if (FactCount >= _maxFacts)
        {
            throw new UnsupportedException("CSHARP_FACT_COUNT_LIMIT", $"C# analysis exceeds {_maxFacts} facts; select a smaller compilation");
        }
        WriteLine(bytes, digest: true);
        FactCount++;
    }

    public void Boundary(string code, SourceAnchors? anchors = null, TextSpan? span = null, string? diagnosticCode = null)
    {
        var row = Row("BOUNDARY");
        row["code"] = code;
        if (diagnosticCode is not null)
        {
            row["diagnosticCode"] = diagnosticCode;
        }
        if (anchors is not null && span is { } location)
        {
            anchors.Apply(row, location);
        }
        row["requiredChecks"] = new JsonArray("VERIFY_CSHARP_COMPILER_RESOLUTION");
        row["resolution"] = "UNKNOWN";
        Write(row);
    }

    /// <summary>Aggregated boundaries without a location, emitted once with a count.</summary>
    public void CountBoundary(string code, int count = 1)
    {
        _droppedBoundaries[code] = _droppedBoundaries.GetValueOrDefault(code) + count;
    }

    public void Finish()
    {
        foreach (var (code, count) in _droppedBoundaries)
        {
            var row = Row("BOUNDARY");
            row["code"] = code;
            row["count"] = count;
            row["requiredChecks"] = new JsonArray("VERIFY_CSHARP_COMPILER_RESOLUTION");
            row["resolution"] = "UNKNOWN";
            Write(row);
        }
        _droppedBoundaries.Clear();
        var trailer = new JsonObject
        {
            ["kind"] = "TRAILER",
            ["factCount"] = FactCount,
            ["factsDigest"] = "sha256:" + Convert.ToHexStringLower(_digest.GetHashAndReset()),
        };
        WriteLine(Encoding.UTF8.GetBytes(trailer.ToJsonString()), digest: false);
        _stream.Flush();
    }

    /// <summary>Absent optional values are omitted rather than serialized as null.</summary>
    private static void StripNulls(JsonNode? node)
    {
        switch (node)
        {
            case JsonObject value:
                foreach (var key in value.Where(pair => pair.Value is null).Select(pair => pair.Key).ToList())
                {
                    value.Remove(key);
                }
                foreach (var (_, child) in value)
                {
                    StripNulls(child);
                }
                break;
            case JsonArray array:
                foreach (var child in array)
                {
                    StripNulls(child);
                }
                break;
        }
    }

    private void WriteLine(byte[] bytes, bool digest)
    {
        _stream.Write(bytes);
        _stream.WriteByte((byte)'\n');
        if (digest)
        {
            _digest.AppendData(bytes);
            _digest.AppendData("\n"u8);
        }
    }

    public void Dispose()
    {
        _stream.Dispose();
        _digest.Dispose();
    }
}
