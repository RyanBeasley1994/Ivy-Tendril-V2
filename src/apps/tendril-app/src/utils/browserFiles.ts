/**
 * Picking and reading files where there is no native dialog. A browser hands over bytes rather than
 * paths, so a picked file is staged with `bridge.uploadAttachmentBytes` instead of by path.
 */

/** Opens the browser's file picker; resolves with nothing if it is dismissed. */
export function pickFiles(): Promise<File[]> {
  return new Promise((resolve) => {
    const input = document.createElement("input");
    input.type = "file";
    input.multiple = true;
    input.addEventListener("change", () => resolve(Array.from(input.files ?? [])));
    input.addEventListener("cancel", () => resolve([]));
    input.click();
  });
}

/** The file's bytes as base64, the form `uploadAttachmentBytes` takes them in. */
export async function fileToBase64(file: Blob): Promise<string> {
  const bytes = new Uint8Array(await file.arrayBuffer());
  let binary = "";
  const chunk = 0x8000;
  for (let i = 0; i < bytes.length; i += chunk) {
    binary += String.fromCharCode(...bytes.subarray(i, i + chunk));
  }
  return btoa(binary);
}
