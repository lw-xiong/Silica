# Quirks

#hashtag at line start is a tag, not a heading.
# Heading with space
#	tab after hash
 # indented heading one space
    # four spaces is code

--- not a break
---
* * *
- - -

1. one
1. one again
1. still one

- a
 - b
  - c
   - d
    - e

- two space nested
  - child
- four space nested
    - child
- tab nested
	- child

Text directly
- followed by a list
> and a quote
# and a heading
```
and code
```

Paragraph then table
| a | b |
| - | - |
| 1 | 2 |

| a |
| - |
| 1 |
paragraph right after table

> quote
followed by lazy paragraph

> [!note] callout
lazy line after callout

>[!tip] No space after marker
>body

> [!tip]Title without space

> [! bad] not a callout

> [!note]  
> Trailing spaces in header

1. item

   para in item

   > quote in item

2. second

[[Links]]: not a definition
[not a link]: because no colon follows
[def]: /x
[def]

Empty lines between

paragraphs are fine

Last line without newline
