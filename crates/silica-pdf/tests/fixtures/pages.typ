// Five A4 pages with an outline; page 3 is landscape. See README.md.
#set document(title: "Pages fixture", author: "silica")
#set page(paper: "a4")
#set text(size: 11pt)

= One
This is page one.

#pagebreak()

== One point one
This is page two.

#page(flipped: true)[
  = Two
  This is page three.
]

== Two point one
This is page four.

#pagebreak()

= Three
This is page five.
