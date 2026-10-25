package li.cil.oc2.os;

import java.io.InputStream;

public final class OpenOs {
    public static InputStream getDiskImage() {
        return open("generated/oc2-os/disk.img");
    }

    private OpenOs() {
    }

    private static InputStream open(final String resource) {
        final InputStream stream = OpenOs.class.getClassLoader().getResourceAsStream(resource);
        if (stream == null) {
            throw new IllegalStateException("Missing resource [" + resource + "].");
        }
        return stream;
    }
}
