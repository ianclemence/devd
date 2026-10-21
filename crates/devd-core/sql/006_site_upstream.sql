-- A site can be served by a program that is already running - a Node, Python or
-- Rails dev server - instead of by a document root. The upstream is the address
-- of that program, e.g. http://127.0.0.1:3000. NULL means "serve the directory".
ALTER TABLE sites ADD COLUMN upstream TEXT;
