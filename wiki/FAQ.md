## General

### Which osu! versions and modes does it support?

osu!stable, standard and catch. osu!lazer, taiko and mania maps aren't supported. You
can still import hitsounds from a mania hitsound diff into a new project.

### Is it safe to use on my mapset?

Direct export backs up every `.osu` file it changes to `<project>.hsproj\backups\`
first. It reads the result back and checks every hit before it writes, and blocks the
export if anything doesn't match. **Hitsound-only copies** never touches the mapset.
It's still an alpha, so keep your own backups of maps you care about.

### Does hitsounds-rs send any data?

Only the update check: one request to GitHub Releases when the app starts, with the
app's version as the user agent. Crash reports are written to a local file and never
sent. Turn the update check off in **App settings** > **Updates**.

### Is this an official osu! tool?

No. It's a fan-made tool and isn't affiliated with osu! or ppy. Its hitsound rules
follow the osu! wiki, Mapping Tools and listening tests in osu!stable.

## Installing and updating

### Windows says "Windows protected your PC".

Alpha builds aren't code signed yet. Choose **More info** > **Run anyway**. See
[[Setting Up|Setting-Up]].

### Installer or portable?

The installer puts hitsounds-rs in your user's programs folder with a Start menu
shortcut. The portable zip runs from any folder you can write to. Both update
themselves and share the same settings.

### The portable build says it can't update.

It needs to write to its own folder. Move it out of a read-only location (such as
`Program Files`), or download new versions from Releases by hand.

### How do I uninstall?

**Settings** > **Apps** > **hitsounds-rs** > **Uninstall**, or delete the portable
folder. To remove your settings as well, delete `%APPDATA%\dev.hitsounds-rs.app`.

## Using it

### Where do the default hitsounds come from?

From a skin folder you choose under **App settings**. The osu! default samples can't be
redistributed, so hitsounds-rs doesn't include them.

### Why doesn't my Songs folder show up?

hitsounds-rs looks for the folder of a standard osu!stable install. If you installed
osu! somewhere unusual, choose the Songs folder in **App settings**. The list refreshes
each time you open it.

### I moved a mapset folder and the project can't find it.

Open the project anyway. hitsounds-rs looks the mapset up in your Songs folder by name;
if it can't find it, the mapset picker opens so you can choose the new location.

### A difficulty shows lots of "Orphaned" or "Empty" issues.

- **Orphaned**: a hit with no object under it in that difficulty. It doesn't play
  there. That's normal for easier difficulties with fewer objects.
- **Empty**: an object with no hit and no fallback. It plays the default hitnormal.

Set a fallback channel, adjust **Match within**, or add hits on that difficulty's tab.
See [[First Project|First-Project#4-check-each-difficulty]].

### Can I edit the map in osu! while hitsounds-rs is open?

Yes. hitsounds-rs watches the mapset and refreshes when you save in the osu! editor.

### Muted slider ends still make a quiet sound.

osu!stable plays a 5 % greenline at about 8 %, so "muted" ends are very quiet rather
than silent. This is a known limitation.

### Separation is slow or unavailable.

Separation runs on the CPU and takes about half the song's length on a 6-core
desktop CPU. It needs the model file you make yourself, and `onnxruntime.dll` next to
`hitsounds-rs.exe`. See [[Drum Separation|Drum-Separation]].

### The app closed without warning.

hitsounds-rs writes `crash.log` when it crashes. Please
[[report it|Reporting-Bugs]].
