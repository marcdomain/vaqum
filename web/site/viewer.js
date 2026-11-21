// Loads the vaqum viewer (WebAssembly) and wraps its exports in a small
// API. Used by the web page (app.js) and by the Node tests.
//
//   const viewer = await createViewer("vaqum-viewer.wasm"); // or wasm bytes
//   const result = viewer.open(archiveBytes, password);     // JSON-ish object
//   viewer.file(i) / viewer.filePath(i) / viewer.zipAll()

const NONE = 0xffffffff; // usize::MAX on wasm32

export async function createViewer(source) {
  let bytes = source;
  if (typeof source === "string" || source instanceof URL) {
    const response = await fetch(source);
    if (!response.ok) throw new Error(`failed to load ${source}: ${response.status}`);
    bytes = await response.arrayBuffer();
  }
  const { instance } = await WebAssembly.instantiate(bytes, {});
  const ex = instance.exports;
  const encoder = new TextEncoder();
  const decoder = new TextDecoder();

  const take = (len) => {
    len >>>= 0;
    if (len === NONE) return null;
    return new Uint8Array(ex.memory.buffer, ex.vq_out_ptr(), len).slice();
  };
  const put = (data) => {
    const ptr = ex.vq_alloc(data.length);
    new Uint8Array(ex.memory.buffer, ptr, data.length).set(data);
    return ptr;
  };

  return {
    /** Opens an archive; returns {ok, entries, ...} or {ok:false, error, needsPassword}. */
    open(archive, password = "") {
      const a = archive instanceof Uint8Array ? archive : new Uint8Array(archive);
      const pw = encoder.encode(password);
      const ap = put(a);
      const pp = put(pw);
      try {
        return JSON.parse(decoder.decode(take(ex.vq_open(ap, a.length, pp, pw.length))));
      } finally {
        ex.vq_free(ap, a.length);
        ex.vq_free(pp, pw.length);
      }
    },
    fileCount() {
      return ex.vq_file_count() >>> 0;
    },
    /** Bytes of downloadable file `i`. */
    file(i) {
      return take(ex.vq_file(i));
    },
    /** Archive path of downloadable file `i`. */
    filePath(i) {
      const b = take(ex.vq_file_path(i));
      return b && decoder.decode(b);
    },
    /** Every file as one .zip. */
    zipAll() {
      return take(ex.vq_zip_all());
    },
  };
}
