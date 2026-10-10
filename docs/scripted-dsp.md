# Scripted effects

nulamp has a built-in effect, the *signal processor*, that runs small scripts on every sample
(spec 08 §5, optional feature). Pick a script in **Options → Plugins** (rows named
"Signal processor: …") and press **Use/Stop**. Only one effect is active at a time; it runs
after the equalizer.

**Configure** opens the script in your text editor. Editing a built-in script first copies it to
`<config>/dsp/`, where the copy replaces the built-in one from the next start. Any `.nsp` file
in `<config>/dsp/` appears in the list. Changes take effect when the script is selected again or
nulamp restarts.

## File format

```text
#name Stereo width
#slider1 1.5 Width (0 = mono, 1 = unchanged)
// Comments start with two slashes.
@init
// Runs when the script starts and when the sample rate or channel count changes.
@slider
// Runs after @init and whenever a slider moves.
@sample
m = (left + right) * 0.5;
s = (left - right) * 0.5 * slider1;
left = m + s;
right = m - s;
```

- `#name` names the script (default: the file name). `#slider1` … `#slider4` give a slider's
  start value and label.
- `@init`, `@slider` and `@sample` start the three code sections. Each holds expressions
  separated by `;`.

## Language

- Numbers are 64-bit floats (`1`, `0.5`, `.5`, `1e-3`). Comparisons and logic give 1 or 0;
  any non-zero value is true.
- Variables need no declaration: they start at 0 and keep their value. Names ignore case.
- Predefined variables: `left`, `right` (the current samples; write them to change the output),
  `srate` (sample rate), `nch` (channel count), `slider1` … `slider4`, and the constant `pi`.
- Operators, lowest precedence first: `=`, `+=`, `-=`, `*=`, `/=` (right to left);
  `c ? a : b`; `||`; `&&`; `==` `!=` `<` `<=` `>` `>=`; `+` `-`; `*` `/` `%`; unary `-` `!`;
  `^` (power, right to left). Division and remainder by zero give 0.
- `buf[i]` is an array of 65 536 values shared by all sections (the index is floored and wraps).
- Functions: `sin cos tan asin acos atan atan2 sqrt pow exp log log10 abs min max floor ceil
  round sign clamp(x, lo, hi) rand(n)` (`rand(n)` is uniform in `[0, n)`).
- Mono input runs with `right = left` and keeps `left`. Channels after the second pass through
  unchanged. A sample that becomes NaN or infinite is output as 0.

## Built-in scripts

Vocal removal, Mono, Swap channels, Stereo width, Echo (delay and feedback sliders), Gain (dB).
Their source is in `crates/nulamp-script/presets/`.
