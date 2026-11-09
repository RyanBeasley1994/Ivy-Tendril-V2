/**
 * A short two-note chime, made with Web Audio so there is no sound file to ship. Quiet on purpose:
 * it says "something came in", not "drop everything".
 */

let ctx: AudioContext | null = null;

export const playChime = (): void => {
  try {
    ctx ??= new AudioContext();
    void ctx.resume();
    const start = ctx.currentTime;
    for (const [i, freq] of [880, 1318.5].entries()) {
      const at = start + i * 0.12;
      const osc = ctx.createOscillator();
      const gain = ctx.createGain();
      osc.type = "sine";
      osc.frequency.value = freq;
      gain.gain.setValueAtTime(0, at);
      gain.gain.linearRampToValueAtTime(0.12, at + 0.01);
      gain.gain.exponentialRampToValueAtTime(0.0001, at + 0.5);
      osc.connect(gain).connect(ctx.destination);
      osc.start(at);
      osc.stop(at + 0.55);
    }
  } catch {
    // No audio device, or the webview refused: the badge still shows it.
  }
};
