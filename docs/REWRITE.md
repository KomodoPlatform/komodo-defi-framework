# Market Maker 2

This document will help us track the information related to the MarketMaker Rust rewrite.

## Rewrite goals

Rewrites and ports
[are costly](http://nibblestew.blogspot.com/2017/04/why-dont-you-just-rewrite-it-in-x.html).
We tend to think that porting is simple, and on some level this intuition is true
because we can skip some high-level design decisions and focus on translating the existing logic,
but it still takes a lot of time
and though we don't need to make some of the new design decisions,
we might spend no less effort to reverse-engineer and understand the old ones.

So why the rewrite then?

Carol, in her talk about rewrites, offers some possible reasons:

"*If* you have some code in C or another language, and need to change it, or it’s slow, or it crashes a
lot, or no one understands it anymore, THEN maybe a rewrite in Rust would be a good fit.
I would also posit that more people are *able* to write production Rust than production C, so if your
team *is* willing to learn Rust, it might actually expand the number of
maintainers." - https://github.com/carols10cents/rust-out-your-c-talk, https://www.youtube.com/watch?v=SKGVItFlK3w.

And we have some of these:

* We *need to change* the MarketMaker:
A faster parallel API.
A more transparent operation, telling user what's going on and what went wrong.
Solve more things automatically, not requiring advanced fine-tuning from users, [a better UTXO (splitting/merging) algorithm in particular](https://github.com/artemii235/SuperNET/issues/157).
Ways to more easily deploy it at home, behind NAT and on small computers (like on a spare mobile phone or on a Raspberry Pi 3).
Running not only Alice but also Bob with GUI.
Ability to embed the MarketMaker in the GUI applications.

* The MarketMaker *crashes a lot*,
to quote hyperDEX: "The biggest issue with the MM right now, is bobs crash or does not have the orders in users orderbook, or when users try to do a order it doesnt work or goes unmatched or other random stuff"
and lukechilds: "We've frequently experienced crashes while querying all swaps with swapstatus".
We want it to be stable and reliable instead.

## Purely functional core

One of our goals is to make the MarketMaker 2 more
[stable and reliable](https://softwareengineering.stackexchange.com/questions/158054/stability-vs-reliability).
We want it to crash less often.
If there was a failure, we want to simplify and preferably automate recovery.
And we want to reduce the time between a failure and a fix.

We'll make a big step towards these goals if the core of the MarketMaker is purely functional.
That is, if we can untangle the *state* of the MarketMaker from the code operating on that state.

The benefits we want to reap from this are:
* Transparency. Some bugs are hard to fix because we don't have enough information about them. We might be lucky to have been running the program in a debugger or finding the necessary bits it verbose logs, but more often than not this is not the case: we know that a failure has happened, but have no idea where and on what input. Separating the state from the code allows the state to be easily shared with a developer, which means much faster roundtrips between discovering a failure and fixing it.
* Replayability. Having a separate state allows us to easily replay any operation. If a failure occured in the middle of a transaction, it should be possible to try a new version of the code without manually repeating all the steps that were necessary to initiate the transaction. And the updated code will run exactly on the failing transaction, not on some other transaction initiated at a later time, which means that users will benefit from less friction and developers will have a better chance to fix the hard-to-reproduce bugs.
* Testability. Stateless code is much easier to test and according to Kent Beck is often a natural result of a Test-Driven development process.
* Portability. Separating the state from the code allows us to more easily use the stateless parts from the sandboxed environments, such as when running under the Web Assembly (WASM). We only need to port the state-managing layer, fully reusing the stateless code.
* Hot reloading. When the code is separated from state, it's trivial to reload it, both with the shared libraries in CPU-native environments (dlopen) and with WASM in GUI environments. This might positively affect the development cycle, reducing the round-trip time from a failure to a fix.
* Concurrency. MarketMaker can currently only perform a single API operation at the time. The more stateless code we have the easier it should be to introduce the parallel execution of API requests in the future.

Implementation might consist of two layers.
A layer that is ported to the host environment (native, JS, Java, Objective-C, cross-compiled Raspberry Pi 3, etc) and implements the TCP/IP communication, state management, hot reloading, all things that won't fit into the WASM sandbox.
And a layer that implements the core logic in a stateless manner and which is compiled into a native shared library or, in the future, to WASM.

Parts of the state might be marked as sensitive.
This will give the users an option to share only the information that can be freely shared,
without a risk of loosing money that is.
Even without the sensitive information a state snapshot might provide the developer with enough data to quickly triage and/or fix the failure, therefore improving the roundtrip time before a failure and a fix.
Plus users will more easily share their problems when it's quick, automatic and doesn't pose a monetary risk.

TODO: Dig the [Event Sourcing](https://martinfowler.com/eaaDev/EventSourcing.html) and see if it highlights the hidden sides of such a design.

The feasibility of this approach is yet to be evaluated, but we can move gradually towards it
by separating the code into the stateful and stateless layers while working on the basic Rust port.

During the initial Rust port we're going to  
a) Mark the ported functions as purely functional or stateful, allowing us to more easily focus on the state management code in the future.  
b) Where possible, take a low-hanging fruit and try to refactor the functions towards being stateless.  
c) Probably mark as stateful the `sleep`ing functions, because `sleep` can be seen as changing the global state (moving the time forwards) and might negatively affect Transparency (we have no idea what's going on while a function is sleeping), Testability (sleeping tests might kill the TDD development cycle), Portability (sleeps are not compatible with WASM), Hot reloading and Concurrency (let's say we want to load new version of the code, but the old version is still sleeping somewhere).

## Gradual rewrite

Above in the [Rewrite goals](#rewrite-goals) section we have identified some of the goals that we pursue with this rewrite.
These goals constitute the Value (in the Lean Production terms) that we are going to create.

For a project to succeed it is usually important to make shorter the path the Value takes to the users.
(Inventory is waste. If we have created the Value but the users can't get their hands on it, we're wasting that Value).

Hence we're going to start with a gradual rewrite. Keeping the version under rewrite immediately avaliable to the users willing to experiment with it.

Let's list the good things that should come out of the gradual rewrite:
* Transparency. With the second version of the MarketMaker being immediately available we can always check the Value we're getting. Is it more stable? Does it have new functions? Or did we hit the wall? What's going on with the code and how can we help? Gradual rewrite is critical for transparency because the change is available in small increments. We can more easily see what function has changed or what new functionality was added when we aren't being uprooted from the familiar context.
* Risk reduction. It comes with transparency, as we can now more easily measure the progress, identify the roadblocks as they occur, see certain problems when they occur and not months after. Plus a gradual rewrite will by default follow the outline of the original project. We have a working system and we're improving it piece by piece, having the original design to fall back to. This makes it less likely for the rewrite to suffer from far-reaching redesign flaws (cf. [Second-system effect](https://en.wikipedia.org/wiki/Second-system_effect)) and creative blocks (cf. [Pantsing](https://www.wikiwrimo.org/wiki/Pantsing)).
* Feedback. Incorporating user feedback is critical for most projects out there, allowing us to iteratively improve the functionality in the right direction (cf. [Fail faster](https://www.youtube.com/watch?v=rDjrOaoHz9s), [PDIA](https://www.youtube.com/watch?v=ZKdjBbiGjao)). The more early we get the feedback, the more time we have to react, and at a lesser cost.
* Motivation. Feedback is not only important to guide us, but also to show us that our work is meaningful and changes lives to the better. It is the cornerstone of Agile (["Build projects around motivated individuals"](https://www.agilealliance.org/agile101/12-principles-behind-the-agile-manifesto/)) and affects our performance on all levels, down to the physical health.

The plan so far is to by default use the C functions as the atomic units of rewrite.
Rust FFI allows us to swap any C function with a similar Rust function.
Porting on this level we
* reuse the function-level modularity of the C language;
* preserve the code meta-data (Git history will show a nice diff between the C and Rust functions, we'll be able to easily investigate the evolution of the code back to its roots);
* avoid the complexity and slow-downs associated with adding RPC/networking layers or drawing new lines of abstraction;
* have a good indicator of the porting progress (how many functions were ported, how many remains).

Focusing on the function call chains that are a common part of a failure/crash or touch on the new functionality
will allow us to leverage the [Pareto principle](https://en.wikipedia.org/wiki/Pareto_principle),
advancing on 80% of desired Value (stability, functionality) with 20% of initial effort.