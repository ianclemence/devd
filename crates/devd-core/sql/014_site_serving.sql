-- What a directory site serves, when a person has said so: 'php' or 'static'.
--
-- NULL means nobody chose, and devd reads the directory to find out (see
-- serving.rs). That is deliberate: existing rows are left exactly as they were,
-- and a site is never given a content type just because one is installed or is
-- the default. A proxy site has an upstream and no content type.
ALTER TABLE sites ADD COLUMN serving TEXT;
