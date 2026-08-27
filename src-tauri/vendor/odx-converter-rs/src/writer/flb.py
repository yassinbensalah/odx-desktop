import lzma
import blackboxprotobuf
with open("test.mdd", "rb") as f:
    data = f.read()

# Remove MDD header
if data[:16] == b"MDD version 0   ":
    payload = data[20:]
else:
    payload = data[4:]

# Decode protobuf container
message, _ = blackboxprotobuf.decode_message(payload)

# Get chunk
chunk = message["6"]

# Get compressed FlatBuffer
compressed = chunk["8"]

# Decompress LZMA
flatbuffer_data = lzma.decompress(
    compressed,
    format=lzma.FORMAT_ALONE
)

# Save raw FlatBuffer
output = r"C:\Users\YassinBs\Desktop\odx-converter-rs-cf-fixed\test.bin"

with open(output, "wb") as f:
    f.write(flatbuffer_data)

print("Saved:", output)
print("FlatBuffer size:", len(flatbuffer_data))