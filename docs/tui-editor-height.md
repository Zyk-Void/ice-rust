# Long drafts in the editor

The editor keeps at most 30% of the terminal's rows for draft text, with a
minimum of five text rows. Longer drafts scroll within the editor so the
transcript remains visible. The borders show how many rows are hidden above
and below; the working indicator keeps priority in the top border during a run.

The viewport follows the caret through wrapped lines and is recalculated on
terminal resize. Up and Down continue to move by visual row, including wide
characters and combining text. Automated regressions cover wrapped boundaries,
Unicode, resizing, and returning to a short draft.

For terminal verification, enter a draft several screens long, navigate across
wrapped boundaries, resize the terminal in both directions, and confirm that
the caret remains visible while the transcript retains space above the editor.