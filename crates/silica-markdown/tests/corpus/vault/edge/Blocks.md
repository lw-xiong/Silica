---
title: Blocks
tags: [edge, blocks]
---
# Headings

## Deeper

### Level three #tag-in-heading

#### Level *four* with `code`

##### Level five

###### Level six

####### Not a heading

#hashtag-not-heading

Setext one
==========

Setext two
----------

A paragraph with a block id. ^para1

Another paragraph
spanning two lines ^para2

---

***

___

> A quote
> continued lazily
with a lazy line

> Nested
> > inner quote
> > > deepest

> [!note]
> Default note callout

> [!tip] A titled tip
> With body

> [!warning]- Collapsed warning
> Hidden by default

> [!danger]+ Expanded danger
> Shown by default

> [!abstract] Alias types
> [!summary] inside is just text

> [!custom] Unknown type
> Renders as note styling

> [!faq]- Nested
> > [!question] Inner callout
> > Inner body
>
> - list in callout
> - with [[Links]]

> [!tip] Title with **bold** and [[Links|a link]] and #callout-tag

> [!note]
> ```js
> let code = "in a callout";
> ```

- Tight item one
- Tight item two
  - Nested two spaces
    - Nested four spaces
- Tight item three

- Loose item one

- Loose item two

1. Ordered one
2. Ordered two
3. Ordered three

7. Starts at seven
8. Eight

1) Paren delimiter
2) Second

- [ ] Open task
- [x] Done task
- [X] Capital done
- [?] Question task
- [-] Cancelled task
- [>] Forwarded task
- [ ] Task with [[Links]] and #task-tag
    - [ ] Nested task
- [ ]Not a task without space
- [ ]

* Star bullet
+ Plus bullet

- Item with paragraph

  Second paragraph in item

  ```
  code in item
  ```

```rust
fn main() {
    println!("[[not a link]] #nottag ==nothighlight==");
}
```

~~~python
print("tilde fence")
~~~

```
no language
```

````md
```
nested fence
```
````

    indented code block
    second line

<div class="custom">
  <p>HTML block</p>
</div>

<!-- an html comment -->

$$
\int_0^1 x^2 \, dx = \frac{1}{3}
$$

$$ a^2 + b^2 = c^2 $$

%%
A block comment
spanning lines
%%

%% single line block comment %%

| Left | Center | Right | None |
| :--- | :----: | ----: | ---- |
| a | b | c | d |
| **bold** | *em* | `code` | [[Links]] |
| short |
| too | many | cells | here | extra |

Table without outer pipes:

Name | Value
--- | ---
x | 1

| Table with id |
| --- |
| row |

^tableid

- list with id

^listid

```
code with id
```

^codeid

Term with trailing backslash\
hard break and two spaces  
hard break.

Final paragraph. ^final
