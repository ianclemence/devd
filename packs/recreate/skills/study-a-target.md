---
name: study-a-target
description: Turn a live target into observations a builder can work from
kind: discover
roles: recreator,researcher,designer,frontend,mobile
---

# Study a target

The study pass produces evidence, not opinions. Everything you record is something a tool
showed you, with the tool and the width named, so the build can be defended later.

## When to use

At the start of a recreation, and again whenever a flow or a state is missing from the capture.

## Procedure

1. Write the target down: the address or the app, and the two or three things the person
   actually uses it for. `recreate.begin` records this beside the run.
2. Open it and take a picture at three widths - a phone, a tablet, a desktop - with
   `recreate.capture`. One width is not the layout; the layouts differ and the differences
   are what the build gets wrong.
3. Read the page as a user would with `browser.snapshot`: headings in order, the controls and
   what they are called, the fields, the links, what is above the fold.
4. Walk the flows the person named. At each step capture again and note what changed: a new
   screen, a dialog, a state with nothing in it. Empty and error states are part of the app.
5. Record the look as measurements, not adjectives: the palette, the type scale, the spacing,
   the radius, the shadow. `browser.screenshot` plus a short note per screen is enough.
6. Put the observations in the project's capture folder through `recreate.capture` and
   `project.write`: one screen per note, each naming its width and its step.

## Pitfalls

- One desktop screenshot standing in for the whole product.
- Recording what a screen means instead of what it shows; the builder needs the second.
- Forgetting the second and third states of a screen: signed out, empty, failed.

## Verification

The capture folder has three widths per screen, every flow the person named has a picture per
step, and each note names the tool and the wait that produced it.
