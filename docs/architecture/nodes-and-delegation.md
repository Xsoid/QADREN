# Nodes and Delegation

Status: **accepted** at the principle level. Interaction protocol is **deferred**.

## Terms: source is not equal to Controller

`StorageConnection` means a connection to a cloud or a server. In the rules, such a connection is a **source** or a **destination** of data. `Controller` and `Worker` are roles of **QADREN nodes**, not storages. Each adapter can have an unlimited number of connection instances; a node processes only connections that are explicitly permitted. Authorization keys of one Worker are not transferred to the Controller automatically (see [D-016](../decisions/D-016-multiple-connections.md)).

## One program — different roles

- **Controller** creates rules, assigns tasks, receives reports, and shows status.
- **Worker** performs work with files and clouds.
- An instance can combine roles. A subordinate Mac can run the same program as the main Mac.
- The main MacBook does not need access to the content of the backed-up data.

## Task ownership

After delegation, the assigned Worker is the **only executor** of the task. The Controller:
- does not start a parallel transfer;
- does not download a file to check a delegated task;
- does not perform a retry instead of the Worker without a separate explicit re-assignment process;
- accepts, saves, and displays the Worker report.

If the connection is lost, the task must not be considered as not executed: the state must reflect uncertainty (for example, `UNKNOWN`), and the retry transfer must be designed as idempotent.

## Reports

The report must distinguish between the Worker statement "success", the confirmation from the remote service, and a confirmed integrity check. Report data can include task ID, version identifier, source and destination, result, write completion signs, hash/size, errors, and time. The exact schema is not defined now.

## Autonomy

After the Worker receives permissions and rules, the Worker must be able to execute permitted tasks when the Controller is off. This is a **target capability**; the specific implementation depends on the communication mechanism and permissions.

## Not designed now

Discovery, pairs and trust between nodes, transport, authentication, distributed locking, log synchronization, delegation revocation, and the policy for selecting a new Worker. Resolve these concerns separately before implementation.
